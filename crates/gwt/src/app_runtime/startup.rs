//! Bootstrap / startup auto-resume split out of `app_runtime/mod.rs` for
//! SPEC-3064 Phase 1 (Pass 2).
//!
//! Owns:
//! - [`AppRuntime::bootstrap`] (one-shot startup work: retroactive merge
//!   migration, recovery-session restore queueing, ingest kicks)
//! - The startup auto-resume queue and its geometry / freshness helpers
//!   ([`AppRuntime::queue_startup_auto_resume_sessions`],
//!   [`AppRuntime::startup_auto_resume_ready_events`],
//!   `startup_auto_resume_window_geometry`, `startup_auto_resume_is_fresh`,
//!   `mark_auto_resume_source_completed`, ...)
//! - Restoring open-project windows / paused placeholders
//!   ([`AppRuntime::restore_open_project_windows`],
//!   [`AppRuntime::spawn_restored_agent_session`])
//! - Late runtime wiring setters ([`AppRuntime::set_agent_capability_issuer`],
//!   [`AppRuntime::set_server_url`], [`AppRuntime::set_usage_refresh`])
//!
//! Behavior-preserving move: `AppRuntime::new` and
//! `PendingStartupAutoResumeSession` stay in `mod.rs`.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration as StdDuration,
};

use sha2::{Digest, Sha256};

use crate::{
    launch_runtime::{
        prune_orphan_intake_worktrees_with_protected, repair_unresolved_intake_recovery_base_pins,
    },
    runtime_support::is_ephemeral_intake_worktree,
};

use super::launch::launch_checkpoint_continuation_config;

use super::{
    combined_window_id, launch_config_from_persisted_session, prune_orphan_intake_worktrees,
    same_worktree_path, should_auto_start_restored_window, workspace_resume_context_for_work_item,
    AgentCapabilityIssuer, AppRuntime, OutboundEvent, PendingProviderRootClaim,
    PendingStartupAutoResumeSession, UserEvent, WindowGeometry, WindowPreset, WindowProcessStatus,
    WorkspaceResumeContext,
};

/// SPEC-3214 T-006: per-repo cap on orphaned intake worktrees reaped per
/// startup so a pathological pile-up cannot stall boot.
const MAX_STARTUP_INTAKE_PRUNE: usize = 32;
const STARTUP_AUTO_RESUME_STALE_AFTER_SECS: i64 = 24 * 60 * 60;
const STARTUP_AUTO_RESUME_STACK_OFFSET_X: f64 = 28.0;
const STARTUP_AUTO_RESUME_STACK_OFFSET_Y: f64 = 24.0;
const STARTUP_RECOVERY_LEASE_TTL_MINUTES: i64 = 5;
const PROVIDER_ROOT_CLAIM_RENEW_INTERVAL_SECS: u64 = 30;
// Keep automatic fallback identical to the explicit Recovery Center action:
// newest 12 visible items, capped at 12k characters.
const CONTINUATION_MAX_VISIBLE_ITEMS: usize = 12;
const CONTINUATION_MAX_CHARS: usize = 12_000;

const ATTENTION_MISSING_CONTEXT: &str = "Durable checkpoint continuation context is unavailable";
const ATTENTION_LAUNCH_FAILED: &str = "Recovery launch failed before provider readiness";
const ATTENTION_PROVIDER_STOPPED: &str = "Recovery provider stopped before readiness";
const ATTENTION_WORKTREE_RECREATE_FAILED: &str =
    "Pinned Intake worktree could not be recreated safely";
const ATTENTION_INDETERMINATE_SPAWN: &str =
    "A provider spawn was requested before the previous process ended; automatic relaunch is disabled to avoid a duplicate provider";
const INTERRUPTED_EXPIRED_LEASE_REASON: &str =
    "Startup confirmed the recovery Session was interrupted after its launch lease expired";
const INTERRUPTED_SUPERVISOR_STOP_REASON: &str =
    "gwt confirmed that no live PTY supervisor owns this Intake provider";
const LEGACY_MISSING_INTAKE_ATTENTION: &str = "legacy_import_attention:missing_intake_worktree";
const LEGACY_MISSING_PROVIDER_ROOT_ATTENTION: &str =
    "legacy_import_attention:missing_provider_root";
const CONTINUATION_REASON_NO_EXACT_ROOT: &str =
    "Exact provider resume is unavailable; continuing from durable checkpoint context";
const CONTINUATION_REASON_DEFINITIVE_REJECTION: &str =
    "Exact provider resume rejected: No conversation found with session ID";
const CONTINUATION_REASON_STARTUP_EXACT: &str = "Startup requested exact provider resume";

fn recovery_store_for_session(session: &gwt_agent::Session) -> gwt_core::recovery::RecoveryStore {
    let project_root = session
        .project_state_root
        .as_deref()
        .unwrap_or(&session.worktree_path);
    gwt_core::recovery::RecoveryStore::for_project_dir(
        gwt_core::paths::gwt_project_dir_for_repo_path(project_root),
    )
}

fn recovery_repo_id_for_session(session: &gwt_agent::Session) -> Option<String> {
    let project_root = session
        .project_state_root
        .as_deref()
        .unwrap_or(&session.worktree_path);
    project_root
        .exists()
        .then(|| gwt_core::paths::project_scope_hash(project_root).to_string())
}

fn recovery_kind_for_session(
    session: &gwt_agent::Session,
) -> Option<gwt_core::recovery::RecoverySessionKind> {
    match (session.session_kind, session.is_ephemeral) {
        (Some(gwt_skills::SessionKind::Intake), true) => {
            Some(gwt_core::recovery::RecoverySessionKind::Intake)
        }
        (Some(gwt_skills::SessionKind::Execution), false) => {
            Some(gwt_core::recovery::RecoverySessionKind::Execution)
        }
        _ => None,
    }
}

fn recovery_session_can_be_interrupted_after_supervisor_stop(session: &gwt_agent::Session) -> bool {
    matches!(
        session.status,
        gwt_agent::AgentStatus::Running
            | gwt_agent::AgentStatus::Idle
            | gwt_agent::AgentStatus::WaitingInput
            | gwt_agent::AgentStatus::Interrupted
            | gwt_agent::AgentStatus::Stopped
    ) && recovery_kind_for_session(session).is_some()
}

fn recovery_launch_stage_has_known_spawned_process(
    stage: gwt_core::recovery::RecoveryLaunchStage,
) -> bool {
    (gwt_core::recovery::RecoveryLaunchStage::ProcessSpawned
        ..=gwt_core::recovery::RecoveryLaunchStage::Ready)
        .contains(&stage)
}

fn recovery_was_interrupted_after_supervisor_stop(
    session: &gwt_agent::Session,
    record: &gwt_core::recovery::RecoveryRecord,
) -> bool {
    record.lifecycle == gwt_core::recovery::RecoveryLifecycle::Interrupted
        && recovery_has_matching_supervisor_stop_proof(session, record)
}

fn recovery_has_matching_supervisor_stop_proof(
    session: &gwt_agent::Session,
    record: &gwt_core::recovery::RecoveryRecord,
) -> bool {
    recovery_launch_stage_has_known_spawned_process(record.launch_stage)
        && record
            .supervisor_stop_proof
            .as_ref()
            .is_some_and(|proof| proof.session_id == session.id)
}

fn recovery_attention_can_record_supervisor_stop(
    session: &gwt_agent::Session,
    record: &gwt_core::recovery::RecoveryRecord,
) -> bool {
    if record.lifecycle != gwt_core::recovery::RecoveryLifecycle::Attention
        || record.session_kind != gwt_core::recovery::RecoverySessionKind::Intake
        || session.session_kind != Some(gwt_skills::SessionKind::Intake)
        || !session.is_ephemeral
        || !recovery_launch_stage_has_known_spawned_process(record.launch_stage)
    {
        return false;
    }
    record.lifecycle_reason.as_deref().is_some_and(|reason| {
        reason == LEGACY_MISSING_INTAKE_ATTENTION
            || reason == LEGACY_MISSING_PROVIDER_ROOT_ATTENTION
            || reason == ATTENTION_WORKTREE_RECREATE_FAILED
            || reason
                .strip_prefix(ATTENTION_WORKTREE_RECREATE_FAILED)
                .is_some_and(|suffix| suffix.starts_with(": "))
    })
}

/// Reconcile a persisted Recovery Session only when this AppRuntime has proved
/// that no live PTY is registered for it.
///
/// Windows providers are attached to a kill-on-close Job Object; Unix
/// providers own a PTY whose controlling master belongs to this process. A
/// cold AppRuntime therefore cannot inherit the old provider. We persist that
/// supervisor boundary before enabling either exact or semantic successor
/// claims. A merely stale Session status is never sufficient by itself.
fn reconcile_stopped_session_recovery(
    session: &gwt_agent::Session,
    supervisor_stopped: bool,
) -> Result<bool, String> {
    if !supervisor_stopped || !recovery_session_can_be_interrupted_after_supervisor_stop(session) {
        return Ok(false);
    }
    let Some(recovery_id) = session.recovery_id.as_deref() else {
        return Ok(false);
    };
    let store = recovery_store_for_session(session);
    let Some(mut record) = store
        .load(recovery_id)
        .map_err(|error| format!("load interrupted recovery {recovery_id}: {error}"))?
    else {
        return Ok(false);
    };
    if record.session_id != session.id
        || recovery_kind_for_session(session) != Some(record.session_kind)
    {
        return Ok(false);
    }

    let now = chrono::Utc::now();
    if record.lifecycle == gwt_core::recovery::RecoveryLifecycle::Recovering {
        let Some(lease) = record.recovery_lease.as_ref() else {
            return Ok(false);
        };
        if lease.expires_at > now {
            // Another gwt instance may still own this bounded claim. Local
            // runtime absence cannot override its durable lease.
            return Ok(false);
        }
        record = store
            .interrupt_expired_recovery_lease(
                recovery_id,
                now,
                INTERRUPTED_EXPIRED_LEASE_REASON,
                format!(
                    "startup-interrupt-expired-lease:{}:{}",
                    session.id, lease.lease_id
                ),
            )
            .map_err(|error| format!("interrupt expired recovery lease {recovery_id}: {error}"))?;
    }

    if recovery_attention_can_record_supervisor_stop(session, &record) {
        let expected_attention_reason = record
            .lifecycle_reason
            .as_deref()
            .expect("retryable Attention requires a classified reason");
        return store
            .interrupt_attention_after_supervisor_stop(
                recovery_id,
                record.generation,
                &session.id,
                expected_attention_reason,
                now,
                INTERRUPTED_SUPERVISOR_STOP_REASON,
                format!(
                    "startup-attention-supervisor-stop-v1:{}:{}",
                    session.id, record.generation
                ),
            )
            .map(|_| true)
            .map_err(|error| {
                format!(
                    "record stopped supervisor for retryable Attention recovery {recovery_id}: {error}"
                )
            });
    }

    if recovery_was_interrupted_after_supervisor_stop(session, &record) {
        return Ok(true);
    }
    if !matches!(
        record.lifecycle,
        gwt_core::recovery::RecoveryLifecycle::Launching
            | gwt_core::recovery::RecoveryLifecycle::Running
            | gwt_core::recovery::RecoveryLifecycle::Interrupted
    ) || !recovery_launch_stage_has_known_spawned_process(record.launch_stage)
    {
        return Ok(false);
    }

    store
        .interrupt_after_supervisor_stop(
            recovery_id,
            record.generation,
            &session.id,
            now,
            INTERRUPTED_SUPERVISOR_STOP_REASON,
            format!(
                "startup-supervisor-stop-v1:{}:{}",
                session.id, record.generation
            ),
        )
        .map(|_| true)
        .map_err(|error| format!("record stopped supervisor for recovery {recovery_id}: {error}"))
}

fn reconcile_startup_recovery_supervisors(
    sessions_dir: &Path,
    sessions: &[gwt_agent::Session],
    live_supervised_session_ids: &HashSet<String>,
) {
    for session in sessions {
        let supervisor_stopped = !live_supervised_session_ids.contains(&session.id);
        match reconcile_stopped_session_recovery(session, supervisor_stopped) {
            Ok(true) => {
                // RecoveryStore proof is the launch authority; Session is the
                // user-visible projection. Publish it second so a crash can
                // only leave a retryable stale Session marker.
                if let Err(error) = gwt_agent::persist_session_status(
                    sessions_dir,
                    &session.id,
                    gwt_agent::AgentStatus::Interrupted,
                ) {
                    tracing::warn!(
                        session_id = %session.id,
                        error = %error,
                        "failed to project stopped recovery into Session state"
                    );
                }
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(
                    session_id = %session.id,
                    recovery_id = ?session.recovery_id,
                    %error,
                    "failed to reconcile a stopped recovery supervisor at startup"
                );
            }
        }
    }
}

fn recovery_session_identity_hash(repo_id: &str, session_id: &str, recovery_id: &str) -> String {
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(
            format!("{repo_id}:{session_id}:{recovery_id}").as_bytes()
        ))
    )
}

fn recovery_attention_allows_missing_intake_retry(
    session: &gwt_agent::Session,
    record: &gwt_core::recovery::RecoveryRecord,
) -> bool {
    if record.lifecycle != gwt_core::recovery::RecoveryLifecycle::Attention
        || record.session_kind != gwt_core::recovery::RecoverySessionKind::Intake
        || session.session_kind != Some(gwt_skills::SessionKind::Intake)
        || !session.is_ephemeral
    {
        return false;
    }
    record.lifecycle_reason.as_deref().is_some_and(|reason| {
        reason == LEGACY_MISSING_INTAKE_ATTENTION
            || reason == ATTENTION_WORKTREE_RECREATE_FAILED
            || reason
                .strip_prefix(ATTENTION_WORKTREE_RECREATE_FAILED)
                .is_some_and(|suffix| suffix.starts_with(": "))
    })
}

/// A legacy Intake can lack any exact provider root while still retaining a
/// bounded user-visible checkpoint. A pre-spawn record is safe to continue;
/// after ProcessSpawned, the same path additionally requires durable proof
/// that its PTY supervisor stopped. Other Attention reasons remain manual: in
/// particular ambiguous roots and indeterminate SpawnRequested outcomes never
/// pass.
fn recovery_attention_allows_checkpoint_continuation(
    session: &gwt_agent::Session,
    record: &gwt_core::recovery::RecoveryRecord,
) -> bool {
    record.lifecycle == gwt_core::recovery::RecoveryLifecycle::Attention
        && record.lifecycle_reason.as_deref() == Some(LEGACY_MISSING_PROVIDER_ROOT_ATTENTION)
        && (record.launch_stage < gwt_core::recovery::RecoveryLaunchStage::SpawnRequested
            || recovery_has_matching_supervisor_stop_proof(session, record))
        && record.session_id == session.id
        && record.session_kind == gwt_core::recovery::RecoverySessionKind::Intake
        && session.session_kind == Some(gwt_skills::SessionKind::Intake)
        && session.is_ephemeral
        && session.restore_window_on_startup
        && session.exact_resume_session_id().is_none()
}

fn startup_exact_auto_resume_candidate_without_worktree(session: &gwt_agent::Session) -> bool {
    matches!(
        session.status,
        gwt_agent::AgentStatus::Running
            | gwt_agent::AgentStatus::Idle
            | gwt_agent::AgentStatus::WaitingInput
            | gwt_agent::AgentStatus::Interrupted
    ) && (session.last_hook_event_at.is_some() || session.last_completed_stop_at.is_some())
        && session.exact_resume_session_id().is_some()
}

/// Managed Recovery v2 does not depend on provider hook timestamps to prove a
/// cold-start interruption. The RecoveryStore supervisor-stop mutation is the
/// stronger evidence: it is generation-CAS committed only after AppRuntime
/// established that no live PTY owns this Recovery Session.
fn startup_managed_recovery_resume_candidate(session: &gwt_agent::Session) -> Result<bool, String> {
    let Some(recovery_id) = session.recovery_id.as_deref() else {
        return Ok(false);
    };
    let store = recovery_store_for_session(session);
    let Some(record) = store
        .load(recovery_id)
        .map_err(|error| format!("load managed recovery evidence {recovery_id}: {error}"))?
    else {
        return Ok(false);
    };
    Ok(record.session_id == session.id
        && recovery_kind_for_session(session) == Some(record.session_kind)
        && (recovery_was_interrupted_after_supervisor_stop(session, &record)
            || recovery_attention_allows_checkpoint_continuation(session, &record)))
}

/// Restore, or re-verify after a crash, the narrowly-scoped Intake worktree
/// used by automatic startup recovery. `Ok(false)` means this Session is not
/// an eligible missing-Intake auto-resume and must retain the historical skip
/// behavior. No Execution/user path reaches the Git recreation primitive.
fn prepare_startup_intake_worktree(
    session: &gwt_agent::Session,
    active_project_root: &Path,
) -> Result<bool, String> {
    let target_exists = session.worktree_path.exists();
    if session.session_kind != Some(gwt_skills::SessionKind::Intake) || !session.is_ephemeral {
        return Ok(false);
    }
    let Some(recovery_id) = session
        .recovery_id
        .as_deref()
        .map(str::trim)
        .filter(|recovery_id| !recovery_id.is_empty())
    else {
        return if target_exists {
            Ok(false)
        } else {
            Err("missing Intake Session has no recovery identity".to_string())
        };
    };
    let project_root = session
        .project_state_root
        .as_deref()
        .filter(|path| path.is_dir())
        .or_else(|| {
            target_exists
                .then_some(active_project_root)
                .filter(|path| path.is_dir())
        })
        .ok_or_else(|| "missing Intake Session has no available Project State root".to_string())?;
    let session_repo_root = gwt_git::worktree::main_worktree_root(project_root)
        .map_err(|error| format!("resolve Intake recovery repository: {error}"))?;
    let active_repo_root = gwt_git::worktree::main_worktree_root(active_project_root)
        .map_err(|error| format!("resolve active project repository: {error}"))?;
    if !same_worktree_path(&session_repo_root, &active_repo_root) {
        return Err("Intake recovery repository does not match the active project".to_string());
    }

    let store = recovery_store_for_session(session);
    let record = store
        .load(recovery_id)
        .map_err(|error| format!("load missing Intake recovery {recovery_id}: {error}"))?
        .ok_or_else(|| format!("missing Intake recovery {recovery_id} has no durable record"))?;
    let expected_runtime = match session.runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => "host",
        gwt_agent::LaunchRuntimeTarget::Docker => "docker",
    };
    let project_repo_id = gwt_core::paths::project_scope_hash(project_root).to_string();
    let active_repo_id = gwt_core::paths::project_scope_hash(active_project_root).to_string();
    let session_base_conflicts = session
        .launch_base_oid
        .as_deref()
        .map(str::trim)
        .filter(|oid| !oid.is_empty())
        .is_some_and(|oid| !oid.eq_ignore_ascii_case(&record.launch_base_oid));
    if record.session_id != session.id
        || record.repo_id != project_repo_id
        || record.repo_id != active_repo_id
        || session
            .repo_hash
            .as_deref()
            .is_some_and(|repo_id| repo_id != record.repo_id)
        || record.session_kind != gwt_core::recovery::RecoverySessionKind::Intake
        || !same_worktree_path(&record.worktree_path, &session.worktree_path)
        || record.provider != session.agent_id.to_string()
        || record.runtime != expected_runtime
        || session_base_conflicts
    {
        return Err(format!(
            "missing Intake recovery {recovery_id} identity does not match its Session and active project"
        ));
    }
    if record.launch_stage == gwt_core::recovery::RecoveryLaunchStage::SpawnRequested
        || record.launch_stage.is_terminal()
        || matches!(
            record.lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Resolved
                | gwt_core::recovery::RecoveryLifecycle::Discarded
        )
    {
        return Ok(false);
    }
    if record.lifecycle == gwt_core::recovery::RecoveryLifecycle::Attention
        && !recovery_attention_allows_missing_intake_retry(session, &record)
        && !recovery_attention_allows_checkpoint_continuation(session, &record)
    {
        return Ok(false);
    }
    if record
        .recovery_lease
        .as_ref()
        .is_some_and(|lease| lease.expires_at > chrono::Utc::now())
    {
        return Ok(false);
    }
    if let Some(exact_root_id) = session.exact_resume_session_id() {
        if !record.provider_root.as_ref().is_some_and(|root| {
            root.quality.is_authoritative() && root.root_id.trim() == exact_root_id.trim()
        }) {
            return Ok(false);
        }
    } else if !recovery_has_matching_supervisor_stop_proof(session, &record) {
        // A provider process can be durably known to have spawned before its
        // root id was observed. The cold supervisor boundary makes a bounded
        // checkpoint/original-intent continuation safe; every other rootless
        // Intake remains ineligible for automatic launch.
        return Ok(false);
    }

    if target_exists {
        gwt_git::recovery::verify_recovery_intake_worktree(
            project_root,
            &record.worktree_path,
            recovery_id,
            &record.launch_base_oid,
        )
        .map_err(|error| format!("verify existing Intake recovery worktree: {error}"))?;
    } else {
        gwt_git::recovery::recreate_missing_intake_worktree(
            project_root,
            &record.worktree_path,
            recovery_id,
            &record.launch_base_oid,
        )
        .map_err(|error| format!("recreate pinned Intake recovery worktree: {error}"))?;
    }
    Ok(true)
}

/// Gate exact startup resume against the durable recovery owner when one
/// exists. Session provider ids remain useful history, but cannot override an
/// Attention/terminal lifecycle, an active claim, or a matching tombstone.
fn startup_exact_resume_allowed_by_recovery_sot(
    session: &gwt_agent::Session,
    exact_root_id: &str,
) -> Result<bool, String> {
    if session
        .recovery_launch_stage
        .is_some_and(gwt_agent::session::RecoveryLaunchStage::is_terminal)
    {
        return Ok(false);
    }
    let fallback_recovery_id;
    let recovery_id = if let Some(recovery_id) = session
        .recovery_id
        .as_deref()
        .map(str::trim)
        .filter(|recovery_id| !recovery_id.is_empty())
    {
        recovery_id
    } else {
        fallback_recovery_id = format!("legacy-{}", session.id);
        &fallback_recovery_id
    };
    let store = recovery_store_for_session(session);
    if let Some(record) = store
        .load(recovery_id)
        .map_err(|error| format!("load recovery {recovery_id}: {error}"))?
    {
        if record.launch_stage == gwt_core::recovery::RecoveryLaunchStage::SpawnRequested {
            return Ok(false);
        }
        let session_repo_id = recovery_repo_id_for_session(session).ok_or_else(|| {
            format!("recovery {recovery_id} Session repo identity is unavailable")
        })?;
        let expected_runtime = match session.runtime_target {
            gwt_agent::LaunchRuntimeTarget::Host => "host",
            gwt_agent::LaunchRuntimeTarget::Docker => "docker",
        };
        let expected_kind = session.session_kind.map(|kind| match kind {
            gwt_skills::SessionKind::Intake => gwt_core::recovery::RecoverySessionKind::Intake,
            gwt_skills::SessionKind::Execution => {
                gwt_core::recovery::RecoverySessionKind::Execution
            }
        });
        if record.session_id != session.id
            || record.repo_id != session_repo_id
            || !same_worktree_path(&record.worktree_path, &session.worktree_path)
            || record.provider != session.agent_id.to_string()
            || record.runtime != expected_runtime
            || expected_kind.is_some_and(|kind| record.session_kind != kind)
        {
            return Err(format!(
                "recovery {recovery_id} identity does not match source Session {}",
                session.id
            ));
        }
        if matches!(
            record.lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Resolved
                | gwt_core::recovery::RecoveryLifecycle::Discarded
        ) || (record.lifecycle == gwt_core::recovery::RecoveryLifecycle::Attention
            && !recovery_attention_allows_missing_intake_retry(session, &record))
        {
            return Ok(false);
        }
        if record
            .recovery_lease
            .as_ref()
            .is_some_and(|lease| lease.expires_at > chrono::Utc::now())
        {
            return Ok(false);
        }
        if recovery_launch_stage_has_known_spawned_process(record.launch_stage)
            && !recovery_has_matching_supervisor_stop_proof(session, &record)
        {
            return Ok(false);
        }
        let exact_root_id = exact_root_id.trim();
        return Ok(record.provider_root.as_ref().is_some_and(|root| {
            root.quality.is_authoritative()
                && !exact_root_id.is_empty()
                && root.root_id.trim() == exact_root_id
        }));
    }

    let Some(tombstone) = store
        .load_tombstone(recovery_id)
        .map_err(|error| format!("load recovery tombstone {recovery_id}: {error}"))?
    else {
        // Startup legacy import reconstructs every eligible pre-store Session
        // before this gate. A missing owner after that inventory is incomplete
        // evidence, not permission to bypass the recovery SOT.
        return Ok(false);
    };
    let repo_id = recovery_repo_id_for_session(session).ok_or_else(|| {
        format!("recovery tombstone {recovery_id} Session repo identity is unavailable")
    })?;
    let expected_identity = recovery_session_identity_hash(&repo_id, &session.id, recovery_id);
    if tombstone.session_identity_hash != expected_identity {
        return Err(format!(
            "recovery tombstone {recovery_id} identity does not match source Session {}",
            session.id
        ));
    }
    Ok(false)
}

fn checkpoint_continuation_prompt_for_runtime(
    session: &gwt_agent::Session,
    store: &gwt_core::recovery::RecoveryStore,
    record: &gwt_core::recovery::RecoveryRecord,
) -> Result<String, String> {
    let has_attachments = record
        .checkpoint
        .as_ref()
        .is_some_and(|checkpoint| !checkpoint.attachment_refs.is_empty());
    if session.runtime_target != gwt_agent::LaunchRuntimeTarget::Docker || !has_attachments {
        return gwt_core::recovery::build_checkpoint_continuation_prompt_with_attachments(
            store,
            record,
            CONTINUATION_MAX_VISIBLE_ITEMS,
            CONTINUATION_MAX_CHARS,
        )
        .map_err(|error| {
            format!(
                "build recovery {} continuation: {error}",
                record.recovery_id
            )
        });
    }
    if session.agent_id != gwt_agent::AgentId::Codex {
        return Err(
            "Docker checkpoint attachments require the Codex private sidecar staging path"
                .to_string(),
        );
    }

    // Never put a Host RecoveryStore path into a Docker-bound prompt. This is
    // a short-lived path-free seed; launch preparation replaces it with the
    // bounded prompt containing sidecar-verified container paths before the
    // target recovery is created or either Codex process starts.
    match gwt_core::recovery::build_checkpoint_continuation_prompt(
        record,
        CONTINUATION_MAX_VISIBLE_ITEMS,
        CONTINUATION_MAX_CHARS,
    ) {
        Ok(prompt) => Ok(prompt),
        Err(gwt_core::recovery::RecoveryStoreError::NoContinuationContext { .. }) => Ok(format!(
            "Continue the interrupted {} session using its verified user-provided attachments.",
            match record.session_kind {
                gwt_core::recovery::RecoverySessionKind::Intake => "Intake",
                gwt_core::recovery::RecoverySessionKind::Execution => "Execution",
            }
        )),
        Err(error) => Err(format!(
            "build recovery {} continuation: {error}",
            record.recovery_id
        )),
    }
}

/// Build the only permitted no-exact-id automatic launch: a Normal provider
/// root fed by bounded, durable, user-visible recovery context. `Ok(None)`
/// means there is no active recovery owner and preserves the legacy diagnostic
/// behavior; `Err` means an active recovery exists but cannot be continued and
/// therefore needs operator attention.
fn prepare_checkpoint_continuation(
    session: &gwt_agent::Session,
    definitive_reason: &str,
) -> Result<Option<gwt_agent::LaunchConfig>, String> {
    prepare_checkpoint_continuation_inner(session, definitive_reason, false)
}

fn prepare_checkpoint_continuation_after_exact_rejection(
    session: &gwt_agent::Session,
) -> Result<Option<gwt_agent::LaunchConfig>, String> {
    let recovery_id = session
        .recovery_id
        .as_deref()
        .ok_or_else(|| "rejected exact-resume Session has no recovery identity".to_string())?;
    let is_exact_successor = session
        .recovery_continuation
        .as_ref()
        .is_some_and(|handoff| {
            handoff.target_recovery_id == recovery_id
                && !handoff.inherit_checkpoint
                && handoff.reason == CONTINUATION_REASON_STARTUP_EXACT
        });
    if !is_exact_successor {
        return Err(
            "definitive-rejection fallback is not backed by the prepared exact successor"
                .to_string(),
        );
    }
    prepare_checkpoint_continuation_inner(session, CONTINUATION_REASON_DEFINITIVE_REJECTION, true)
}

fn prepare_checkpoint_continuation_inner(
    session: &gwt_agent::Session,
    _definitive_reason: &str,
    allow_definitively_rejected_spawn: bool,
) -> Result<Option<gwt_agent::LaunchConfig>, String> {
    let Some(recovery_id) = session.recovery_id.as_deref() else {
        return Ok(None);
    };
    let store = recovery_store_for_session(session);
    let Some(record) = store
        .load(recovery_id)
        .map_err(|error| format!("load recovery {recovery_id}: {error}"))?
    else {
        return Ok(None);
    };
    if record.launch_stage == gwt_core::recovery::RecoveryLaunchStage::SpawnRequested
        && !allow_definitively_rejected_spawn
    {
        return Err(format!(
            "recovery {recovery_id} has an indeterminate prior provider spawn"
        ));
    }
    if matches!(
        record.lifecycle,
        gwt_core::recovery::RecoveryLifecycle::Resolved
            | gwt_core::recovery::RecoveryLifecycle::Discarded
    ) || (record.lifecycle == gwt_core::recovery::RecoveryLifecycle::Attention
        && !recovery_attention_allows_checkpoint_continuation(session, &record))
    {
        return Ok(None);
    }
    if !same_worktree_path(&record.worktree_path, &session.worktree_path) {
        return Err(format!(
            "recovery {recovery_id} worktree does not match source Session"
        ));
    }
    if !session.worktree_path.is_dir() {
        return Err(format!(
            "recovery {recovery_id} worktree is unavailable: {}",
            session.worktree_path.display()
        ));
    }
    let prompt = checkpoint_continuation_prompt_for_runtime(session, &store, &record)?;
    // Eligibility remains read-only until the generation-CAS claim directly
    // before launch. In particular, a Ready/Interrupted source must retain its
    // supervisor-stop proof; a preparatory Recovering write would destroy the
    // dedicated interrupted-claim precondition and strand both exact and
    // semantic continuation paths.
    Ok(Some(launch_checkpoint_continuation_config(
        session, &prompt,
    )))
}

fn prepare_startup_successor(
    session: &gwt_agent::Session,
    config: &mut gwt_agent::LaunchConfig,
    reason: &str,
    inherit_checkpoint: bool,
) -> Result<gwt_core::recovery::RecoveryContinuationLink, String> {
    let source_recovery_id = session
        .recovery_id
        .as_deref()
        .ok_or_else(|| "startup recovery source identity is unavailable".to_string())?;
    let store = recovery_store_for_session(session);
    let source = store
        .load(source_recovery_id)
        .map_err(|error| format!("load startup recovery source {source_recovery_id}: {error}"))?
        .ok_or_else(|| format!("startup recovery source {source_recovery_id} disappeared"))?;
    let reason_key = hex::encode(Sha256::digest(reason.as_bytes()));
    let link = store
        .prepare_successor(
            gwt_core::recovery::RecoveryContinuationLink {
                source_recovery_id: source_recovery_id.to_string(),
                target_recovery_id: uuid::Uuid::new_v4().to_string(),
                source_checkpoint_revision: source.checkpoint_revision,
                definitive_reason: reason.to_string(),
                linked_at: chrono::Utc::now(),
            },
            format!(
                "startup-successor-v1:{}:{}:{}",
                source_recovery_id,
                source.checkpoint_revision,
                &reason_key[..16]
            ),
        )
        .map_err(|error| format!("prepare startup recovery successor: {error}"))?;
    config.recovery_continuation = Some(gwt_agent::RecoveryContinuationHandoff {
        source_session_id: session.id.clone(),
        source_recovery_id: source_recovery_id.to_string(),
        target_recovery_id: link.target_recovery_id.clone(),
        source_checkpoint_revision: link.source_checkpoint_revision,
        reason: link.definitive_reason.clone(),
        inherit_checkpoint,
    });
    Ok(link)
}

/// Reconstruct a launch that crashed after the successor Session/Recovery
/// were durably created but before `SpawnRequested`. Reusing both identities
/// makes `create:<session-id>` and continuation publication idempotent; a
/// restarted app must not allocate a successor-of-successor merely because it
/// lost the asynchronous launch completion message.
pub(super) fn prepare_materialized_successor_retry(
    session: &gwt_agent::Session,
) -> Result<Option<gwt_agent::LaunchConfig>, String> {
    let Some(handoff) = session.recovery_continuation.as_ref() else {
        return Ok(None);
    };
    let Some(recovery_id) = session.recovery_id.as_deref() else {
        return Err("successor Session has no recovery identity".to_string());
    };
    if handoff.target_recovery_id != recovery_id {
        return Err("successor Session handoff target identity changed".to_string());
    }
    let store = recovery_store_for_session(session);
    let Some(record) = store
        .load(recovery_id)
        .map_err(|error| format!("load materialized successor {recovery_id}: {error}"))?
    else {
        return Ok(None);
    };
    if record.session_id != session.id {
        return Err(format!(
            "materialized successor {recovery_id} belongs to Session {}, not {}",
            record.session_id, session.id
        ));
    }
    if record.launch_stage > gwt_core::recovery::RecoveryLaunchStage::WorktreeMaterialized {
        return Ok(None);
    }
    let mut config = if session.session_mode == gwt_agent::SessionMode::Resume {
        let provider_root_id = session
            .exact_resume_session_id()
            .ok_or_else(|| "exact successor retry has no provider root".to_string())?;
        if record
            .provider_root
            .as_ref()
            .is_some_and(|root| root.root_id != provider_root_id)
        {
            return Err("exact successor retry provider root changed".to_string());
        }
        launch_config_from_persisted_session(session)
    } else if handoff.inherit_checkpoint {
        let prompt = checkpoint_continuation_prompt_for_runtime(session, &store, &record)?;
        launch_checkpoint_continuation_config(session, &prompt)
    } else {
        let prompt = record.initial_prompt.trim();
        if prompt.is_empty() {
            return Err("fresh successor retry has no original request".to_string());
        }
        launch_checkpoint_continuation_config(session, prompt)
    };
    config.recovery_continuation = Some(handoff.clone());
    config.recovery_retry_session_id = Some(session.id.clone());
    config.recovery_retry_created_at = Some(session.created_at);
    Ok(Some(config))
}

fn has_materialized_successor(session: &gwt_agent::Session) -> Result<bool, String> {
    let Some(source_recovery_id) = session.recovery_id.as_deref() else {
        return Ok(false);
    };
    let store = recovery_store_for_session(session);
    let Some(link) = store
        .prepared_successor_for_source(source_recovery_id)
        .map_err(|error| format!("load prepared successor for {source_recovery_id}: {error}"))?
    else {
        return Ok(false);
    };
    Ok(store
        .load(&link.target_recovery_id)
        .map_err(|error| {
            format!(
                "load prepared successor {}: {error}",
                link.target_recovery_id
            )
        })?
        .is_some()
        || store
            .load_tombstone(&link.target_recovery_id)
            .map_err(|error| {
                format!(
                    "load prepared successor tombstone {}: {error}",
                    link.target_recovery_id
                )
            })?
            .is_some())
}

/// Claim the durable recovery owner immediately before an automatic startup
/// launch. The earlier eligibility checks are intentionally read-only; this
/// generation-CAS lease is the cross-process barrier that prevents two gwt
/// instances from launching the same interrupted session after both rendered
/// the same startup inventory.
pub(super) fn claim_startup_recovery(
    session: &gwt_agent::Session,
) -> Result<Option<String>, String> {
    let fallback_recovery_id;
    let recovery_id = if let Some(recovery_id) = session
        .recovery_id
        .as_deref()
        .map(str::trim)
        .filter(|recovery_id| !recovery_id.is_empty())
    {
        recovery_id
    } else {
        fallback_recovery_id = format!("legacy-{}", session.id);
        &fallback_recovery_id
    };
    let store = recovery_store_for_session(session);
    let record = store
        .load(recovery_id)
        .map_err(|error| format!("load recovery {recovery_id} before startup claim: {error}"))?
        .ok_or_else(|| format!("recovery {recovery_id} disappeared before startup claim"))?;
    if record.launch_stage == gwt_core::recovery::RecoveryLaunchStage::SpawnRequested {
        return Err(format!(
            "recovery {recovery_id} has an indeterminate prior provider spawn"
        ));
    }
    if matches!(
        record.lifecycle,
        gwt_core::recovery::RecoveryLifecycle::Resolved
            | gwt_core::recovery::RecoveryLifecycle::Discarded
    ) || (record.lifecycle == gwt_core::recovery::RecoveryLifecycle::Attention
        && !recovery_attention_allows_missing_intake_retry(session, &record)
        && !recovery_attention_allows_checkpoint_continuation(session, &record))
    {
        return Err(format!(
            "recovery {recovery_id} is no longer launchable ({:?})",
            record.lifecycle
        ));
    }
    let interrupted_spawn = recovery_was_interrupted_after_supervisor_stop(session, &record);
    if recovery_launch_stage_has_known_spawned_process(record.launch_stage)
        && !recovery_has_matching_supervisor_stop_proof(session, &record)
    {
        return Err(format!(
            "recovery {recovery_id} has a known prior provider process without durable supervisor-stop proof"
        ));
    }
    if record.session_kind == gwt_core::recovery::RecoverySessionKind::Intake {
        let project_root = session
            .project_state_root
            .as_deref()
            .unwrap_or(&session.worktree_path);
        gwt_git::recovery::ensure_recovery_base_pin(
            project_root,
            recovery_id,
            &record.launch_base_oid,
        )
        .map_err(|error| {
            format!("pin recovery {recovery_id} base before startup launch: {error}")
        })?;
        gwt_git::recovery::verify_recovery_intake_worktree(
            project_root,
            &record.worktree_path,
            recovery_id,
            &record.launch_base_oid,
        )
        .map_err(|error| {
            format!("verify recovery {recovery_id} Intake before startup launch: {error}")
        })?;
    }

    let acquired_at = chrono::Utc::now();
    let lease_id = uuid::Uuid::new_v4().to_string();
    let lease = gwt_core::recovery::RecoveryLease {
        lease_id: lease_id.clone(),
        holder_id: format!("startup:{}:{lease_id}", session.id),
        acquired_at,
        expires_at: acquired_at + chrono::Duration::minutes(STARTUP_RECOVERY_LEASE_TTL_MINUTES),
    };
    let exact_root = session.exact_resume_session_id();
    let result = if let Some(exact_root) = exact_root {
        if interrupted_spawn {
            store.claim_interrupted_recovery_with_provider_root(
                recovery_id,
                record.generation,
                exact_root,
                false,
                lease,
                &format!("startup-pending:{}", session.id),
                "Automatic startup recovery launch after supervisor stop",
                format!("startup-interrupted-claim-v1:{}:{lease_id}", session.id),
            )
        } else {
            store.claim_recovery_with_provider_root(
                recovery_id,
                record.generation,
                exact_root,
                false,
                lease,
                &format!("startup-pending:{}", session.id),
                "Automatic startup recovery launch",
                format!("startup-recovery-claim-v2:{}:{lease_id}", session.id),
            )
        }
    } else if interrupted_spawn {
        store.claim_interrupted_recovery(
            recovery_id,
            record.generation,
            lease,
            "Automatic semantic recovery launch after supervisor stop",
            format!("startup-interrupted-claim-v1:{}:{lease_id}", session.id),
        )
    } else {
        store.claim_recovery(
            recovery_id,
            record.generation,
            lease,
            "Automatic startup recovery launch",
            format!("startup-recovery-claim-v1:{}:{lease_id}", session.id),
        )
    };
    result
        .map(|_| exact_root.map(|_| lease_id))
        .map_err(|error| format!("claim recovery {recovery_id} for startup: {error}"))
}

fn mark_session_recovery_attention(
    session: &gwt_agent::Session,
    reason: &str,
) -> Result<(), String> {
    let Some(recovery_id) = session.recovery_id.as_deref() else {
        return Ok(());
    };
    let store = recovery_store_for_session(session);
    let Some(record) = store
        .load(recovery_id)
        .map_err(|error| format!("load recovery {recovery_id}: {error}"))?
    else {
        return Ok(());
    };
    if matches!(
        record.lifecycle,
        gwt_core::recovery::RecoveryLifecycle::Resolved
            | gwt_core::recovery::RecoveryLifecycle::Discarded
    ) {
        return Ok(());
    }
    if record.lifecycle == gwt_core::recovery::RecoveryLifecycle::Attention
        && record.lifecycle_reason.as_deref() == Some(reason)
    {
        return Ok(());
    }
    store
        .set_lifecycle(
            recovery_id,
            gwt_core::recovery::RecoveryLifecycle::Attention,
            Some(reason.to_string()),
            format!("attention:{}:{}", session.id, record.generation),
        )
        .map(|_| ())
        .map_err(|error| format!("mark recovery {recovery_id} Attention: {error}"))
}

/// Convert the pre-spawn half of the two-phase OS launch boundary into an
/// explicit operator decision. The prior process may or may not exist, so an
/// automatic retry could create a duplicate provider.
fn quarantine_indeterminate_spawn(session: &gwt_agent::Session) -> Result<bool, String> {
    let fallback_recovery_id;
    let recovery_id = if let Some(recovery_id) = session.recovery_id.as_deref() {
        recovery_id
    } else {
        fallback_recovery_id = format!("legacy-{}", session.id);
        &fallback_recovery_id
    };
    let store = recovery_store_for_session(session);
    let Some(record) = store
        .load(recovery_id)
        .map_err(|error| format!("load recovery {recovery_id}: {error}"))?
    else {
        return Ok(session.recovery_launch_stage
            == Some(gwt_agent::session::RecoveryLaunchStage::SpawnRequested));
    };
    if record.launch_stage != gwt_core::recovery::RecoveryLaunchStage::SpawnRequested {
        return Ok(false);
    }
    if record.lifecycle == gwt_core::recovery::RecoveryLifecycle::Interrupted
        && record
            .supervisor_stop_proof
            .as_ref()
            .is_some_and(|proof| proof.session_id == record.session_id)
    {
        // A same-process Stopped/Error event proves the requested spawn no
        // longer owns a PTY. Cold startup never manufactures this proof for
        // SpawnRequested, so unknown crash windows remain quarantined.
        return Ok(false);
    }
    if !matches!(
        record.lifecycle,
        gwt_core::recovery::RecoveryLifecycle::Attention
            | gwt_core::recovery::RecoveryLifecycle::Resolved
            | gwt_core::recovery::RecoveryLifecycle::Discarded
    ) {
        store
            .set_lifecycle(
                recovery_id,
                gwt_core::recovery::RecoveryLifecycle::Attention,
                Some(ATTENTION_INDETERMINATE_SPAWN.to_string()),
                format!("spawn-requested-attention:{}", session.id),
            )
            .map_err(|error| {
                format!("quarantine indeterminate recovery spawn {recovery_id}: {error}")
            })?;
    }
    Ok(true)
}

fn startup_auto_resume_window_geometry(
    index: usize,
    total: usize,
    bounds: gwt::WindowGeometry,
) -> gwt::WindowGeometry {
    let (width, height) = WindowPreset::Agent.default_size();
    let stack_steps = total.saturating_sub(1) as f64;
    let index = index as f64;
    gwt::WindowGeometry {
        x: bounds.x + (bounds.width - width) / 2.0
            - (stack_steps * STARTUP_AUTO_RESUME_STACK_OFFSET_X) / 2.0
            + index * STARTUP_AUTO_RESUME_STACK_OFFSET_X,
        y: bounds.y + (bounds.height - height) / 2.0
            - (stack_steps * STARTUP_AUTO_RESUME_STACK_OFFSET_Y) / 2.0
            + index * STARTUP_AUTO_RESUME_STACK_OFFSET_Y,
        width,
        height,
    }
}

fn session_project_scope_hash(session: &gwt_agent::Session) -> Option<String> {
    session
        .repo_hash
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            session
                .project_state_root
                .as_deref()
                .filter(|path| path.exists())
                .map(|path| gwt_core::paths::project_scope_hash(path).to_string())
        })
        .or_else(|| {
            session
                .worktree_path
                .exists()
                .then(|| gwt_core::paths::project_scope_hash(&session.worktree_path).to_string())
        })
}

fn startup_auto_resume_is_fresh(
    session: &gwt_agent::Session,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    now.signed_duration_since(session.last_activity_at)
        <= chrono::Duration::seconds(STARTUP_AUTO_RESUME_STALE_AFTER_SECS)
}

fn startup_auto_resume_window_was_open(session: &gwt_agent::Session) -> bool {
    if session.startup_restore_intent_recorded {
        return session.restore_window_on_startup;
    }
    if session.restore_window_on_startup {
        return true;
    }
    // Compatibility for sessions saved before the explicit GUI restore flag
    // existed, and for files already migrated once with that flag defaulted.
    session.status != gwt_agent::AgentStatus::Stopped
}

fn protected_intake_paths_from_sessions(sessions: &[gwt_agent::Session]) -> HashSet<PathBuf> {
    sessions
        .iter()
        .filter(|session| is_ephemeral_intake_worktree(&session.worktree_path))
        .filter(|session| match session.session_kind {
            Some(gwt_skills::SessionKind::Intake) => true,
            // A partially-written v4 record that retained the ephemeral bit
            // is inconsistent but still potentially recoverable. Fail closed.
            Some(gwt_skills::SessionKind::Execution) => session.is_ephemeral,
            // Pre-v4 ledgers cannot prove Intake vs Execution. A detached
            // `.intake-*` path is enough to retain it for explicit recovery.
            None => true,
        })
        .map(|session| session.worktree_path.clone())
        .collect()
}

fn persisted_legacy_session_ids(
    sessions_dir: &Path,
    loaded_sessions: &[gwt_agent::Session],
) -> std::io::Result<BTreeSet<String>> {
    let mut ids = loaded_sessions
        .iter()
        .map(|session| session.id.trim())
        .filter(|session_id| !session_id.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    for path in gwt_agent::discover_session_toml_paths(sessions_dir)? {
        let session_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::trim)
            .filter(|session_id| !session_id.is_empty())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "Session TOML has no valid UTF-8 identity: {}",
                        path.display()
                    ),
                )
            })?;
        // A corrupt ledger still owns its identity. Placeholder migration
        // must not create a competing recovery from weaker projection
        // evidence while the Session inventory fails closed.
        ids.insert(session_id.to_string());
    }
    Ok(ids)
}

fn combine_startup_recovery_protection(
    session_inventory_complete: bool,
    session_paths: &HashSet<PathBuf>,
    store_paths: Option<HashSet<PathBuf>>,
) -> Option<HashSet<PathBuf>> {
    if !session_inventory_complete {
        return None;
    }
    store_paths.map(|store_paths| {
        let mut protected = session_paths.clone();
        protected.extend(store_paths);
        protected
    })
}

fn legacy_placeholder_observed_at(path: &Path) -> chrono::DateTime<chrono::Utc> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(chrono::DateTime::<chrono::Utc>::from)
        .unwrap_or_else(|_| chrono::Utc::now())
}

fn legacy_window_provider(window: &gwt::PersistedWindowState) -> Option<String> {
    window.agent_id.clone().or_else(|| match window.preset {
        WindowPreset::Claude => Some("claude".to_string()),
        WindowPreset::Codex => Some("codex".to_string()),
        _ => None,
    })
}

fn legacy_placeholder_lane(
    lane: gwt::WindowLaneKind,
) -> Option<gwt_core::recovery::RecoverySessionKind> {
    match lane {
        gwt::WindowLaneKind::Intake => Some(gwt_core::recovery::RecoverySessionKind::Intake),
        gwt::WindowLaneKind::Execution => Some(gwt_core::recovery::RecoverySessionKind::Execution),
        gwt::WindowLaneKind::Unknown => None,
    }
}

fn legacy_projection_window_id(tab_id: &str, window_id: &str) -> String {
    if window_id.contains("::") {
        window_id.to_string()
    } else {
        combined_window_id(tab_id, window_id)
    }
}

fn legacy_recovery_placeholders_for_tab(
    tab: &super::ProjectTabRuntime,
    live_session_ids: &HashSet<String>,
    live_window_ids: &HashSet<String>,
) -> (Vec<gwt_agent::LegacyRecoveryPlaceholder>, Option<String>) {
    let (projection, projection_error) =
        match gwt_core::workspace_projection::load_workspace_projection(&tab.project_root) {
            Ok(projection) => (projection, None),
            Err(error) => (
                None,
                Some(format!(
                    "load Workspace projection for {}: {error}",
                    tab.project_root.display()
                )),
            ),
        };
    let projection_path =
        gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&tab.project_root);
    let projection_observed_at = legacy_placeholder_observed_at(&projection_path);
    let workspace_path = gwt::workspace_state_path(&tab.project_root);
    let window_observed_at = legacy_placeholder_observed_at(&workspace_path);
    let mut candidates = Vec::new();

    for window in &tab.workspace.persisted().windows {
        let combined_id = combined_window_id(&tab.id, &window.id);
        let projected_agent = projection.as_ref().and_then(|projection| {
            projection
                .agents
                .iter()
                .filter(|agent| {
                    window
                        .session_id
                        .as_deref()
                        .is_some_and(|session_id| agent.session_id == session_id)
                        || agent.window_id.as_deref().is_some_and(|window_id| {
                            legacy_projection_window_id(&tab.id, window_id) == combined_id
                        })
                })
                .max_by_key(|agent| agent.updated_at)
        });
        let candidate_session_id = window.session_id.clone().or_else(|| {
            projected_agent
                .map(|agent| agent.session_id.trim())
                .filter(|session_id| !session_id.is_empty())
                .map(str::to_string)
        });
        let worktree_path = projected_agent
            .and_then(|agent| agent.worktree_path.clone())
            .or_else(|| {
                projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|git| git.worktree_path.clone())
            })
            .or_else(|| Some(tab.project_root.clone()));
        let kind = if window.preset == WindowPreset::Shell
            || projected_agent.is_some_and(|agent| agent.is_shell_work())
        {
            gwt_agent::LegacyRecoveryPlaceholderKind::Shell
        } else if projected_agent.is_some_and(|agent| {
            agent.status_category == gwt_core::workspace_projection::WorkspaceStatusCategory::Done
        }) {
            gwt_agent::LegacyRecoveryPlaceholderKind::Invalid
        } else if crate::runtime_support::window_is_agent_pane(window) {
            gwt_agent::LegacyRecoveryPlaceholderKind::Agent
        } else {
            gwt_agent::LegacyRecoveryPlaceholderKind::NonAgent
        };
        let runtime_is_live = live_window_ids.contains(&combined_id)
            || candidate_session_id
                .as_ref()
                .is_some_and(|session_id| live_session_ids.contains(session_id));
        candidates.push(gwt_agent::LegacyRecoveryPlaceholder {
            source: gwt_agent::LegacyRecoveryPlaceholderSource::WindowState,
            source_id: combined_id,
            source_path: workspace_path.clone(),
            session_id: candidate_session_id,
            provider: legacy_window_provider(window),
            worktree_path,
            session_kind: legacy_placeholder_lane(window.lane_kind),
            kind,
            runtime_is_live,
            observed_at: window_observed_at,
        });
    }

    if let Some(projection) = projection {
        let mut latest_agents = projection.agents.clone();
        latest_agents.sort_by(|left, right| {
            left.session_id
                .cmp(&right.session_id)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| left.window_id.cmp(&right.window_id))
        });
        latest_agents.dedup_by(|left, right| left.session_id == right.session_id);
        for agent in &latest_agents {
            let source_id = agent
                .window_id
                .as_deref()
                .map(|window_id| legacy_projection_window_id(&tab.id, window_id))
                .unwrap_or_else(|| format!("{}::projection::{}", tab.id, agent.session_id));
            let runtime_is_live = live_session_ids.contains(&agent.session_id)
                || live_window_ids.contains(&source_id);
            let worktree_path = agent
                .worktree_path
                .clone()
                .or_else(|| {
                    projection
                        .git_details
                        .as_ref()
                        .and_then(|git| git.worktree_path.clone())
                })
                .or_else(|| Some(tab.project_root.clone()));
            candidates.push(gwt_agent::LegacyRecoveryPlaceholder {
                source: gwt_agent::LegacyRecoveryPlaceholderSource::WorkspaceProjection,
                source_id,
                source_path: projection_path.clone(),
                session_id: Some(agent.session_id.clone()),
                provider: Some(agent.agent_id.clone()),
                worktree_path,
                session_kind: None,
                kind: if agent.is_shell_work() {
                    gwt_agent::LegacyRecoveryPlaceholderKind::Shell
                } else if agent.status_category
                    == gwt_core::workspace_projection::WorkspaceStatusCategory::Done
                {
                    gwt_agent::LegacyRecoveryPlaceholderKind::Invalid
                } else {
                    gwt_agent::LegacyRecoveryPlaceholderKind::Agent
                },
                runtime_is_live,
                observed_at: agent.updated_at.max(projection_observed_at),
            });
        }
    }

    (candidates, projection_error)
}

fn discover_project_recovery_protection(
    project_root: &Path,
    sessions_dir: &Path,
    placeholders: &[gwt_agent::LegacyRecoveryPlaceholder],
    persisted_session_ids: &BTreeSet<String>,
) -> Option<HashSet<PathBuf>> {
    let project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(project_root);
    let store = gwt_core::recovery::RecoveryStore::for_project_dir(project_dir);
    let expected_repo_id = gwt_core::paths::project_scope_hash(project_root).to_string();

    match store.repair_attachment_inventory() {
        Ok(removed) if removed > 0 => tracing::info!(
            project_root = %project_root.display(),
            removed,
            "removed unpublished recovery attachment bytes left by an interrupted process"
        ),
        Ok(_) => {}
        Err(error) => tracing::warn!(
            project_root = %project_root.display(),
            error = %error,
            "failed to sweep unpublished recovery attachment bytes"
        ),
    }

    if sessions_dir.is_dir() {
        let report =
            gwt_agent::import_legacy_recovery_sessions(sessions_dir, &store, &expected_repo_id);
        if !report.imported_exact.is_empty() {
            tracing::info!(
                project_root = %project_root.display(),
                imported = report.imported_exact.len(),
                "imported exact legacy recovery sessions during startup"
            );
        }
        if !report.imported_attention.is_empty() {
            tracing::warn!(
                project_root = %project_root.display(),
                imported = ?report.imported_attention,
                "legacy recovery sessions require explicit attention"
            );
        }
        if !report.skipped.is_empty() {
            tracing::info!(
                project_root = %project_root.display(),
                skipped = ?report.skipped,
                "skipped ineligible or already-imported legacy recovery sessions"
            );
        }
        if !report.errors.is_empty() {
            // A ledger that could not be inspected may name a worktree absent
            // from every successfully loaded record. Without a complete
            // inventory orphanhood cannot be proven, so disable prune.
            tracing::warn!(
                project_root = %project_root.display(),
                errors = ?report.errors,
                "skipping Intake prune because legacy recovery import is incomplete"
            );
            return None;
        }
    }

    let placeholder_report = gwt_agent::import_legacy_recovery_placeholders(
        placeholders,
        persisted_session_ids,
        &store,
        &expected_repo_id,
    );
    if !placeholder_report.imported_attention.is_empty() {
        tracing::warn!(
            project_root = %project_root.display(),
            imported = ?placeholder_report.imported_attention,
            "legacy window/Workspace placeholders require explicit recovery attention"
        );
    }
    if !placeholder_report.skipped.is_empty() {
        tracing::info!(
            project_root = %project_root.display(),
            skipped = ?placeholder_report.skipped,
            "skipped live, invalid, or already-owned legacy placeholders"
        );
    }
    if !placeholder_report.errors.is_empty() {
        tracing::warn!(
            project_root = %project_root.display(),
            errors = ?placeholder_report.errors,
            "skipping Intake prune because placeholder recovery import is incomplete"
        );
        return None;
    }

    // Tombstones remain readable during legacy import even after their expiry
    // time. That pass first writes a terminal marker back to any surviving
    // Session ledger, so removing a 30-day tombstone cannot resurrect the
    // same legacy source on the next startup.
    if let Err(error) = store.remove_expired_tombstones(chrono::Utc::now()) {
        tracing::warn!(
            project_root = %project_root.display(),
            error = %error,
            "skipping Intake prune because recovery tombstone retention failed"
        );
        return None;
    }

    let records = match store.list() {
        Ok(records) => records,
        Err(error) => {
            tracing::warn!(
                project_root = %project_root.display(),
                error = %error,
                "skipping Intake prune because recovery inventory is incomplete"
            );
            return None;
        }
    };

    for (recovery_id, error) in repair_unresolved_intake_recovery_base_pins(project_root, &records)
    {
        // The RecoveryStore record remains authoritative and protected. A
        // mismatched/missing commit is diagnostic evidence, never permission
        // to move a ref or prune the Intake path.
        tracing::warn!(
            project_root = %project_root.display(),
            recovery_id,
            error,
            "failed to repair Intake recovery base pin"
        );
    }

    let mut protected = HashSet::new();
    for record in records {
        let outbox_replay_failed = if record.board_outbox.is_empty() {
            false
        } else {
            match gwt::cli::flush_recovery_board_outbox(project_root, &store, &record.recovery_id) {
                Ok(()) => false,
                Err(error) => {
                    // The durable outbox remains untouched on failure. Keep
                    // its Intake worktree protected even if the lifecycle was
                    // already terminal, so startup cannot destroy recovery
                    // context while Board publication still needs attention.
                    tracing::warn!(
                        project_root = %project_root.display(),
                        recovery_id = %record.recovery_id,
                        error = %error,
                        "failed to replay recovery Board outbox; keeping recovery protected"
                    );
                    true
                }
            }
        };
        let is_intake_path = record.session_kind == gwt_core::recovery::RecoverySessionKind::Intake
            && is_ephemeral_intake_worktree(&record.worktree_path);
        let is_nonterminal = !matches!(
            record.lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Resolved
                | gwt_core::recovery::RecoveryLifecycle::Discarded
        );
        if is_intake_path && (is_nonterminal || outbox_replay_failed) {
            protected.insert(record.worktree_path);
        }
    }
    Some(protected)
}

pub(super) fn mark_auto_resume_source_completed(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<(), String> {
    let path = sessions_dir.join(format!("{session_id}.toml"));
    let source = gwt_agent::Session::load_and_migrate(&path)
        .map_err(|error| format!("load recovery source Session {session_id}: {error}"))?;
    let mut retained_for_board_delivery = false;
    if let Some(recovery_id) = source.recovery_id.as_deref() {
        let store = recovery_store_for_session(&source);
        let project_root = source
            .project_state_root
            .as_deref()
            .unwrap_or(&source.worktree_path);
        if let Some(record) = store
            .load(recovery_id)
            .map_err(|error| format!("load source recovery {recovery_id}: {error}"))?
        {
            if !record.board_outbox.is_empty() {
                let reason =
                    "Recovery reached provider readiness with unacknowledged Board milestones";
                let _ = store.set_lifecycle(
                    recovery_id,
                    gwt_core::recovery::RecoveryLifecycle::Attention,
                    Some(reason.to_string()),
                    format!("ready-outbox-attention:{session_id}:{}", record.generation),
                );
                // Provider continuity and Board delivery are independent.
                // Stop re-launching the superseded Session, but retain the
                // Recovery payload, coordinator intent, attachment set, and
                // Intake pin until the durable Board outbox is acknowledged.
                retained_for_board_delivery = true;
            } else {
                let prepared_successor =
                    store
                        .prepared_successor_for_source(recovery_id)
                        .map_err(|error| {
                            format!("load prepared successor for source {recovery_id}: {error}")
                        })?;
                let finalize_result = if prepared_successor.is_some() {
                    // The reconciler derives the terminal operation id from
                    // the durable continuation transaction. Runtime and
                    // restart repair must never invent different tombstone
                    // ids for the same source.
                    store.reconcile_successors().and_then(|_| {
                        if store.load(recovery_id)?.is_some() {
                            return Err(
                                gwt_core::recovery::RecoveryStoreError::ContinuationConflict,
                            );
                        }
                        match store.load_tombstone(recovery_id)? {
                            Some(tombstone)
                                if tombstone.lifecycle
                                    == gwt_core::recovery::RecoveryLifecycle::Resolved =>
                            {
                                Ok(tombstone)
                            }
                            _ => Err(gwt_core::recovery::RecoveryStoreError::ContinuationConflict),
                        }
                    })
                } else if record.continuation_targets.is_empty() {
                    // Compatibility for source records created before
                    // prepared-successor transactions existed.
                    store.finalize_and_purge(
                        recovery_id,
                        gwt_core::recovery::RecoveryLifecycle::Resolved,
                        chrono::Utc::now(),
                        format!("legacy-provider-ready-resolve:{session_id}"),
                    )
                } else {
                    Err(gwt_core::recovery::RecoveryStoreError::ContinuationConflict)
                };
                if let Err(error) = finalize_result {
                    let reason = "Recovery content could not be finalized after provider readiness";
                    let _ = store.set_lifecycle(
                        recovery_id,
                        gwt_core::recovery::RecoveryLifecycle::Attention,
                        Some(reason.to_string()),
                        format!(
                            "ready-finalize-attention:{session_id}:{}",
                            record.generation
                        ),
                    );
                    return Err(format!(
                        "finalize source recovery {recovery_id} after provider readiness: {error}"
                    ));
                }
                if record.session_kind == gwt_core::recovery::RecoverySessionKind::Intake {
                    gwt_git::recovery::remove_recovery_base_pin(
                        project_root,
                        recovery_id,
                        &record.launch_base_oid,
                    )
                    .map_err(|error| {
                        format!(
                            "source recovery {recovery_id} is resolved, but its Git base pin could not be removed safely: {error}"
                        )
                    })?;
                }
            }
        } else {
            let tombstone = store.load_tombstone(recovery_id).map_err(|error| {
                format!("load source recovery tombstone {recovery_id}: {error}")
            })?;
            if tombstone.as_ref().is_some_and(|tombstone| {
                tombstone.lifecycle == gwt_core::recovery::RecoveryLifecycle::Resolved
            }) && matches!(source.session_kind, Some(gwt_skills::SessionKind::Intake))
            {
                // Retry the safe ref cleanup after a crash/failure that
                // happened after tombstone publication and payload purge.
                let launch_base_oid = tombstone
                    .as_ref()
                    .and_then(|tombstone| tombstone.launch_base_oid.as_deref())
                    .or(source.launch_base_oid.as_deref())
                    .ok_or_else(|| {
                        format!(
                            "resolved source recovery {recovery_id} has no recorded base OID for pin cleanup"
                        )
                    })?;
                gwt_git::recovery::remove_recovery_base_pin(
                    project_root,
                    recovery_id,
                    launch_base_oid,
                )
                .map_err(|error| {
                    format!("retry resolved recovery {recovery_id} Git base pin cleanup: {error}")
                })?;
            }
        }
    }
    gwt_agent::update_session(sessions_dir, session_id, |session| {
        session.update_status(gwt_agent::AgentStatus::Stopped);
        session.restore_window_on_startup = false;
        session.startup_restore_intent_recorded = true;
        if retained_for_board_delivery {
            if !session
                .recovery_launch_stage
                .is_some_and(gwt_agent::session::RecoveryLaunchStage::is_terminal)
            {
                session.advance_recovery_launch_stage(
                    gwt_agent::session::RecoveryLaunchStage::Ready,
                )?;
            }
        } else {
            session
                .advance_recovery_launch_stage(gwt_agent::session::RecoveryLaunchStage::Resolved)?;
        }
        session.recovery_lease = None;
        Ok(())
    })
    .map(|_| ())
    .map_err(|error| format!("retire recovery source Session {session_id}: {error}"))
}

pub(super) fn repair_completed_successor_sessions(
    sessions_dir: &Path,
    sessions: &[gwt_agent::Session],
) {
    for target_session in sessions {
        let Some(handoff) = target_session.recovery_continuation.as_ref() else {
            continue;
        };
        if handoff.source_session_id.trim().is_empty() {
            continue;
        }
        let store = recovery_store_for_session(target_session);
        if let Err(error) = store.reconcile_successors() {
            tracing::warn!(
                target_session_id = %target_session.id,
                source_session_id = %handoff.source_session_id,
                error = %error,
                "failed to reconcile durable recovery successor during startup"
            );
            continue;
        }
        let successor_ready = store
            .load(&handoff.target_recovery_id)
            .ok()
            .flatten()
            .is_some_and(|target| {
                target.launch_stage >= gwt_core::recovery::RecoveryLaunchStage::Ready
            })
            || store
                .load_tombstone(&handoff.target_recovery_id)
                .ok()
                .flatten()
                .is_some_and(|tombstone| {
                    tombstone.lifecycle == gwt_core::recovery::RecoveryLifecycle::Resolved
                });
        if !successor_ready {
            continue;
        }
        let source_resolved = store
            .load(&handoff.source_recovery_id)
            .ok()
            .flatten()
            .is_none()
            && store
                .load_tombstone(&handoff.source_recovery_id)
                .ok()
                .flatten()
                .is_some_and(|tombstone| {
                    tombstone.lifecycle == gwt_core::recovery::RecoveryLifecycle::Resolved
                });
        let source_retained_for_board = store
            .load(&handoff.source_recovery_id)
            .ok()
            .flatten()
            .is_some_and(|source| !source.board_outbox.is_empty());
        if !source_resolved && !source_retained_for_board {
            continue;
        }
        if let Err(error) =
            mark_auto_resume_source_completed(sessions_dir, &handoff.source_session_id)
        {
            tracing::warn!(
                target_session_id = %target_session.id,
                source_session_id = %handoff.source_session_id,
                error = %error,
                "failed to retire resolved recovery source Session during startup"
            );
        }
    }
}

impl AppRuntime {
    pub(crate) fn bootstrap(&mut self) {
        // Recovery inventory must be complete before any startup orphan reap.
        // The first snapshot protects worktrees while legacy import runs.
        // Legacy sessions with no lane metadata remain protected by their
        // `.intake-*` path. Queueing reloads the ledgers after import so the
        // newly persisted recovery binding participates in exact-reject
        // fallback and later source finalization.
        let (startup_recovery_sessions, startup_session_inventory_error) = match self
            .load_recovery_sessions()
        {
            Ok(sessions) => (sessions, None),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "startup Session inventory is incomplete; recovery import and Intake prune are disabled"
                );
                (Vec::new(), Some(error.to_string()))
            }
        };
        let live_supervised_session_ids = self
            .active_agent_sessions
            .iter()
            .filter(|(window_id, _)| self.runtimes.contains_key(*window_id))
            .map(|(_, active)| active.session_id.clone())
            .collect::<HashSet<_>>();
        reconcile_startup_recovery_supervisors(
            &self.sessions_dir,
            &startup_recovery_sessions,
            &live_supervised_session_ids,
        );
        let session_protected_paths =
            protected_intake_paths_from_sessions(&startup_recovery_sessions);
        let persisted_session_ids =
            persisted_legacy_session_ids(&self.sessions_dir, &startup_recovery_sessions);
        let session_inventory_complete =
            startup_session_inventory_error.is_none() && persisted_session_ids.is_ok();
        if let Err(error) = &persisted_session_ids {
            tracing::warn!(
                error = %error,
                "persisted Session identity inventory is incomplete; weak placeholder import and Intake prune are disabled"
            );
        }
        let persisted_session_ids = persisted_session_ids.unwrap_or_default();
        let live_session_ids = self
            .active_agent_sessions
            .values()
            .map(|session| session.session_id.clone())
            .collect::<HashSet<_>>();
        let live_window_ids = self
            .runtimes
            .keys()
            .chain(self.active_agent_sessions.keys())
            .cloned()
            .collect::<HashSet<_>>();
        let recovery_protection_by_tab = self
            .tabs
            .iter()
            .map(|tab| {
                let (placeholders, placeholder_inventory_error) =
                    legacy_recovery_placeholders_for_tab(tab, &live_session_ids, &live_window_ids);
                let store_paths = session_inventory_complete.then(|| {
                    discover_project_recovery_protection(
                        &tab.project_root,
                        &self.sessions_dir,
                        &placeholders,
                        &persisted_session_ids,
                    )
                });
                let mut protected = combine_startup_recovery_protection(
                    session_inventory_complete,
                    &session_protected_paths,
                    store_paths.flatten(),
                );
                if let Some(error) = placeholder_inventory_error {
                    // Preserve the independently readable window candidates,
                    // but do not prune while a second legacy source could not
                    // be inventoried completely.
                    tracing::warn!(
                        project_root = %tab.project_root.display(),
                        error = %error,
                        "skipping Intake prune because legacy placeholder inventory is incomplete"
                    );
                    protected = None;
                }
                (tab.id.clone(), protected)
            })
            .collect::<HashMap<_, _>>();

        match self.load_recovery_sessions() {
            Ok(startup_recovery_sessions) => {
                // Legacy import above assigns recovery identities after the
                // first cold-start pass. Reconcile those newly materialized
                // records before any exact/semantic eligibility gate reads
                // them; otherwise the safe supervisor boundary exists only
                // in memory and every known-spawn record is skipped.
                reconcile_startup_recovery_supervisors(
                    &self.sessions_dir,
                    &startup_recovery_sessions,
                    &live_supervised_session_ids,
                );
                repair_completed_successor_sessions(&self.sessions_dir, &startup_recovery_sessions);
            }
            Err(error) => tracing::warn!(
                error = %error,
                "skipping successor Session repair because inventory is incomplete"
            ),
        }
        match self.load_recovery_sessions() {
            Ok(startup_recovery_sessions) => {
                self.queue_startup_auto_resume_sessions(startup_recovery_sessions)
            }
            Err(error) => tracing::warn!(
                error = %error,
                "skipping startup auto-resume because Session inventory is incomplete"
            ),
        }

        // SPEC-2359 US-37 / FR-119 / FR-123: One-shot retroactive migration to
        // mark historical merged `work/*` Start Work Workspaces as Done so the
        // Workspace Overview Completed column reflects past completions on the
        // first startup after auto-done emission lands. The scan is idempotent
        // per `work_item_id` and skips silently when journal / work_events
        // files are missing or unreadable.
        let now = chrono::Utc::now();
        for tab in &self.tabs {
            let _ =
                gwt_core::workspace_projection::retroactive_auto_done_scan(&tab.project_root, now);
            // SPEC-2359 US-39 / FR-142..145: backfill Phase U-6 schema
            // additions (`summary`, `created_at`, `creator`,
            // `lifecycle_stage`) on legacy `workspace.json` files. Runs
            // alongside the auto-done scan above with independent helpers
            // and an independent `workspace.migration.json` marker, so the
            // two migrations are exactly-once each and never duplicate work.
            // Errors are silently dropped (`let _ = ...`) so a corrupt or
            // unreadable Workspace cannot block daemon startup.
            let _ = gwt_core::workspace_projection_migration::migrate_workspace_projection_for_repo(
                &tab.project_root,
            );
            // SPEC-2359 Phase W-16 (FR-393): decompose legacy mega-items
            // (pre-W-12 records keyed to one projection UUID fusing dozens of
            // branches) into canonical branch-keyed items so each branch row
            // shows its real title / sessions. Idempotent; must run before
            // the intake/reconcile chain so decomposed branches are not
            // redundantly backfilled.
            let _ = gwt_core::workspace_projection::decompose_legacy_multi_branch_work_items(
                &tab.project_root,
            );
            // SPEC-2359 W-16 (FR-387): cross-machine work events intake.
            // Supersedes the one-shot `rebuild_work_items_from_events_for_repo`
            // migration gate — the intake is a permanently-installed idempotent
            // consumer over the same (and more) sources. Runs on a background
            // thread; its completion event then runs the worktree reconcile
            // (intake → reconcile order) and the merge scan.
            self.spawn_work_events_ingest(tab.project_root.clone(), true);
            // SPEC-2359 Phase W-11 (US-58 / FR-346): one-shot, version-guarded
            // clear of legacy prompt-derived title_summary / current_focus so
            // existing broken titles ("あなたの目的は何ですか" etc.) heal via the
            // display fallback and agent re-authoring. Idempotent via
            // `agent_identity.migration.json`; never re-clears agent-authored
            // values written after the marker.
            let _ = gwt_core::workspace_projection::reset_legacy_agent_identity_for_repo(
                &tab.project_root,
            );
            // Reap only Intake worktrees proven unreferenced after Session and
            // project RecoveryStore discovery. Clean generated assets are
            // disposable; user changes and incomplete inventories fail closed.
            let Some(protected_paths) = recovery_protection_by_tab
                .get(&tab.id)
                .and_then(Option::as_ref)
            else {
                continue;
            };
            let pruned = if protected_paths.is_empty() {
                prune_orphan_intake_worktrees(&tab.project_root, MAX_STARTUP_INTAKE_PRUNE)
            } else {
                prune_orphan_intake_worktrees_with_protected(
                    &tab.project_root,
                    MAX_STARTUP_INTAKE_PRUNE,
                    protected_paths,
                )
            };
            if pruned > 0 {
                tracing::info!(
                    project_root = %tab.project_root.display(),
                    pruned,
                    "reaped orphaned ephemeral intake worktrees on startup"
                );
            }
        }

        let windows = self
            .tabs
            .iter()
            .flat_map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .clone()
                    .into_iter()
                    .map(|window| (tab.id.clone(), window))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for (tab_id, window) in windows {
            if !should_auto_start_restored_window(&window) {
                continue;
            }
            let _ = self.start_window(&tab_id, &window.id, window.preset, window.geometry.clone());
        }
        let _ = self.persist();
    }

    fn queue_startup_auto_resume_sessions(&mut self, mut sessions: Vec<gwt_agent::Session>) {
        self.pending_startup_auto_resume_sessions.clear();
        sessions.sort_by(|left, right| {
            right
                .last_activity_at
                .cmp(&left.last_activity_at)
                .then_with(|| left.id.cmp(&right.id))
        });

        let now = chrono::Utc::now();
        let mut resumed_native_sessions = std::collections::HashSet::new();
        for session in sessions {
            // Issue #2942: a persisted Stopped agent placeholder means the user
            // did not explicitly close the window (closing removes it from the
            // workspace). Such "still open" windows must restore regardless of
            // the session's status drift (e.g. idle-timeout -> Stopped) or age,
            // honoring "restore everything not explicitly closed". Sessions with
            // no placeholder are orphans (the workspace lost the window); keep
            // the conservative status / freshness gates so old, windowless
            // sessions are not resurrected at startup.
            match quarantine_indeterminate_spawn(&session) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(
                        session_id = %session.id,
                        error = %error,
                        "indeterminate provider spawn could not be quarantined; skipping automatic recovery"
                    );
                    continue;
                }
            }
            match has_materialized_successor(&session) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(
                        session_id = %session.id,
                        error = %error,
                        "prepared recovery successor could not be reconciled; skipping source relaunch"
                    );
                    continue;
                }
            }
            let worktree_missing = !session.worktree_path.exists();
            let placeholder_tab = self.paused_placeholder_tab_for_session(&session);
            let managed_recovery_candidate =
                match startup_managed_recovery_resume_candidate(&session) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session.id,
                            error = %error,
                            "managed startup recovery evidence could not be verified"
                        );
                        continue;
                    }
                };
            // Orphan sessions (workspace lost the window) keep the conservative
            // status / freshness gates so old, windowless sessions are not
            // resurrected; placeholder sessions restore regardless (Issue #2942).
            if placeholder_tab.is_none() {
                if !startup_auto_resume_window_was_open(&session) {
                    continue;
                }
                if session.exact_resume_session_id().is_some()
                    && if worktree_missing {
                        !startup_exact_auto_resume_candidate_without_worktree(&session)
                            && !managed_recovery_candidate
                    } else {
                        !session.exact_auto_resume_candidate() && !managed_recovery_candidate
                    }
                {
                    continue;
                }
                if !startup_auto_resume_is_fresh(&session, now) {
                    continue;
                }
            }
            if self
                .active_agent_sessions
                .values()
                .any(|active| active.session_id == session.id)
            {
                continue;
            }
            let Some(tab_id) =
                placeholder_tab.or_else(|| self.auto_resume_tab_id_for_session(&session))
            else {
                continue;
            };
            let Some(tab) = self.tab(&tab_id) else {
                continue;
            };
            if tab.kind != gwt::ProjectKind::Git || tab.migration_pending {
                continue;
            }
            let active_project_root = tab.project_root.clone();
            let intake_preparation_required = worktree_missing
                || (session.session_kind == Some(gwt_skills::SessionKind::Intake)
                    && session.is_ephemeral);
            if intake_preparation_required {
                match prepare_startup_intake_worktree(&session, &active_project_root) {
                    Ok(true) => {}
                    Ok(false) if worktree_missing => continue,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session.id,
                            error = %error,
                            "Intake recovery worktree is not safe to use during startup"
                        );
                        let reason = format!("{ATTENTION_WORKTREE_RECREATE_FAILED}: {error}");
                        if let Err(attention_error) =
                            mark_session_recovery_attention(&session, &reason)
                        {
                            tracing::warn!(
                                session_id = %session.id,
                                error = %attention_error,
                                "failed to mark unsafe Intake worktree recovery Attention"
                            );
                        }
                        continue;
                    }
                }
            }
            let materialized_retry = match prepare_materialized_successor_retry(&session) {
                Ok(retry) => retry,
                Err(error) => {
                    tracing::warn!(
                        session_id = %session.id,
                        error = %error,
                        "materialized recovery successor could not be retried safely"
                    );
                    continue;
                }
            };
            let resume_key = if materialized_retry.is_some() {
                let Some(recovery_id) = session.recovery_id.as_deref() else {
                    continue;
                };
                format!("materialized-successor:{recovery_id}")
            } else if let Some(native_session_id) = session.exact_resume_session_id() {
                match startup_exact_resume_allowed_by_recovery_sot(&session, native_session_id) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session.id,
                            error = %error,
                            "exact startup recovery is not safely attributable"
                        );
                        continue;
                    }
                }
                let config = launch_config_from_persisted_session(&session);
                if config.session_mode != gwt_agent::SessionMode::Resume {
                    continue;
                }
                format!("exact:{native_session_id}")
            } else {
                match prepare_checkpoint_continuation(&session, CONTINUATION_REASON_NO_EXACT_ROOT) {
                    Ok(Some(config)) if config.session_mode == gwt_agent::SessionMode::Normal => {
                        let Some(recovery_id) = session.recovery_id.as_deref() else {
                            continue;
                        };
                        format!("checkpoint:{recovery_id}")
                    }
                    Ok(_) => continue,
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session.id,
                            error = %error,
                            "checkpoint continuation unavailable during startup"
                        );
                        if let Err(attention_error) =
                            mark_session_recovery_attention(&session, ATTENTION_MISSING_CONTEXT)
                        {
                            tracing::warn!(
                                session_id = %session.id,
                                error = %attention_error,
                                "failed to mark startup recovery Attention"
                            );
                        }
                        continue;
                    }
                }
            };
            if !resumed_native_sessions.insert(resume_key) {
                continue;
            }
            let workspace_resume_context = Some(workspace_resume_context_for_work_item(
                &session.worktree_path,
                Some(session.branch.as_str()),
                &session.worktree_path,
            ));
            self.pending_startup_auto_resume_sessions
                .push(PendingStartupAutoResumeSession {
                    tab_id,
                    session,
                    workspace_resume_context,
                    fallback_geometry: None,
                });
        }
    }

    pub(super) fn startup_auto_resume_ready_events(
        &mut self,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        if self.pending_startup_auto_resume_sessions.is_empty() {
            return Vec::new();
        }

        // A reconnect may repeat StartupAutoResumeReady while the first
        // provider is still crossing its Ready barrier. Assigned geometry is
        // the durable-in-runtime marker that this queue has already started.
        if self.pending_startup_auto_resume_sessions[0]
            .fallback_geometry
            .is_some()
        {
            return Vec::new();
        }
        let total = self.pending_startup_auto_resume_sessions.len();
        for (index, pending) in self
            .pending_startup_auto_resume_sessions
            .iter_mut()
            .enumerate()
        {
            pending.fallback_geometry = Some(startup_auto_resume_window_geometry(
                index,
                total,
                bounds.clone(),
            ));
        }
        self.launch_next_startup_auto_resume_session()
    }

    /// Advance the newest-first startup queue by exactly one live provider.
    /// Synchronous failures are already terminal Attention decisions, so they
    /// are skipped immediately; an asynchronous launch stops the loop until
    /// its Ready or Attention transition calls this method again.
    pub(super) fn launch_next_startup_auto_resume_session(&mut self) -> Vec<OutboundEvent> {
        let mut events = Vec::new();
        while !self.pending_startup_auto_resume_sessions.is_empty() {
            let pending = self.pending_startup_auto_resume_sessions.remove(0);
            let source_session_id = pending.session.id.clone();
            let fallback_geometry = pending.fallback_geometry.unwrap_or_else(|| {
                let (width, height) = WindowPreset::Agent.default_size();
                WindowGeometry {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height,
                }
            });
            let mut spawned = self.spawn_restored_agent_session(
                &pending.tab_id,
                pending.session,
                pending.workspace_resume_context,
                fallback_geometry,
            );
            events.append(&mut spawned);
            if self
                .pending_auto_resume_sources
                .values()
                .any(|source| source == &source_session_id)
            {
                break;
            }
        }
        events
    }

    /// Spawn a single restored agent window from a persisted session, reusing
    /// the paused placeholder's geometry when present (Issue #2942). Shared by
    /// startup auto-resume and the Open Project restore path so both honor the
    /// "restore everything the user did not explicitly close" rule. Records the
    /// source session in `pending_auto_resume_sources` so the lifecycle handler
    /// retires the old session once the resumed window reports its own id.
    fn spawn_restored_agent_session(
        &mut self,
        tab_id: &str,
        session: gwt_agent::Session,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        fallback_geometry: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let materialized_retry = match prepare_materialized_successor_retry(&session) {
            Ok(retry) => retry,
            Err(error) => {
                tracing::warn!(
                    session_id = %session.id,
                    error = %error,
                    "materialized recovery successor changed before launch"
                );
                let _ = mark_session_recovery_attention(&session, ATTENTION_LAUNCH_FAILED);
                return Vec::new();
            }
        };
        if let Some(config) = materialized_retry {
            let Some(handoff) = config.recovery_continuation.as_ref() else {
                return Vec::new();
            };
            let source_path = self
                .sessions_dir
                .join(format!("{}.toml", handoff.source_session_id));
            let mut source = match gwt_agent::Session::load_and_migrate(&source_path) {
                Ok(source) => source,
                Err(error) => {
                    tracing::warn!(
                        target_session_id = %session.id,
                        source_session_id = %handoff.source_session_id,
                        error = %error,
                        "materialized successor source Session is unavailable"
                    );
                    let _ = mark_session_recovery_attention(&session, ATTENTION_MISSING_CONTEXT);
                    return Vec::new();
                }
            };
            if source.recovery_id.as_deref() != Some(handoff.source_recovery_id.as_str())
                || !same_worktree_path(&source.worktree_path, &session.worktree_path)
            {
                let _ = mark_session_recovery_attention(&session, ATTENTION_MISSING_CONTEXT);
                return Vec::new();
            }
            let exact_retry = config.session_mode == gwt_agent::SessionMode::Resume;
            if !exact_retry {
                // A semantic/fresh successor never claims the superseded
                // provider conversation, even when its source Session still
                // carries an exact-resume id.
                source.agent_session_id = None;
                source.session_mode = gwt_agent::SessionMode::Normal;
            }
            let provider_root_claim_token = match claim_startup_recovery(&source) {
                Ok(token) => token,
                Err(error) => {
                    tracing::warn!(
                        target_session_id = %session.id,
                        source_session_id = %source.id,
                        error = %error,
                        "materialized successor source claim was rejected"
                    );
                    return Vec::new();
                }
            };
            return self.spawn_restored_agent_session_with_config(
                tab_id,
                source,
                config,
                workspace_resume_context,
                fallback_geometry,
                provider_root_claim_token,
            );
        }
        let exact_resume = session.exact_resume_session_id().is_some();
        let mut config = if exact_resume {
            launch_config_from_persisted_session(&session)
        } else {
            match prepare_checkpoint_continuation(&session, CONTINUATION_REASON_NO_EXACT_ROOT) {
                Ok(Some(config)) => config,
                Ok(None) => return Vec::new(),
                Err(error) => {
                    tracing::warn!(
                        session_id = %session.id,
                        error = %error,
                        "checkpoint continuation became unavailable before launch"
                    );
                    if let Err(attention_error) =
                        mark_session_recovery_attention(&session, ATTENTION_MISSING_CONTEXT)
                    {
                        tracing::warn!(
                            session_id = %session.id,
                            error = %attention_error,
                            "failed to mark recovery Attention"
                        );
                    }
                    return Vec::new();
                }
            }
        };
        let provider_root_claim_token = match claim_startup_recovery(&session) {
            Ok(token) => token,
            Err(error) => {
                tracing::warn!(
                    session_id = %session.id,
                    error = %error,
                    "startup recovery claim was rejected"
                );
                return Vec::new();
            }
        };
        let successor_reason = if exact_resume {
            CONTINUATION_REASON_STARTUP_EXACT
        } else {
            CONTINUATION_REASON_NO_EXACT_ROOT
        };
        if let Err(error) =
            prepare_startup_successor(&session, &mut config, successor_reason, !exact_resume)
        {
            tracing::warn!(
                session_id = %session.id,
                error = %error,
                "startup recovery successor could not be prepared"
            );
            if let (Some(claim_token), Some(recovery_id)) = (
                provider_root_claim_token.as_deref(),
                session.recovery_id.as_deref(),
            ) {
                let store = recovery_store_for_session(&session);
                let _ = store.release_provider_root_claim_for_recovery(
                    recovery_id,
                    claim_token,
                    chrono::Utc::now(),
                );
            }
            let _ = mark_session_recovery_attention(&session, ATTENTION_LAUNCH_FAILED);
            return Vec::new();
        }
        self.spawn_restored_agent_session_with_config(
            tab_id,
            session,
            config,
            workspace_resume_context,
            fallback_geometry,
            provider_root_claim_token,
        )
    }

    fn spawn_restored_agent_session_with_config(
        &mut self,
        tab_id: &str,
        session: gwt_agent::Session,
        config: gwt_agent::LaunchConfig,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        fallback_geometry: WindowGeometry,
        provider_root_claim_token: Option<String>,
    ) -> Vec<OutboundEvent> {
        let geometry = self
            .remove_stale_paused_agent_window(tab_id, &session.id)
            .unwrap_or(fallback_geometry);
        // Snapshot the window registry *after* the paused placeholder is
        // removed: the freshly spawned window may reuse the placeholder's id
        // (ids are assigned lowest-free), so a pre-removal snapshot would fail
        // to detect it and the source session would never be retired.
        let existing_windows = self
            .window_lookup
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        match self.spawn_agent_window_at_geometry(
            tab_id,
            config,
            geometry,
            workspace_resume_context,
        ) {
            Ok(events) => {
                if let Some(window_id) = self
                    .window_lookup
                    .keys()
                    .find(|window_id| !existing_windows.contains(*window_id))
                    .cloned()
                {
                    self.pending_auto_resume_sources
                        .insert(window_id.clone(), session.id.clone());
                    if let (Some(claim_token), Some(recovery_id)) = (
                        provider_root_claim_token.as_ref(),
                        session.recovery_id.as_ref(),
                    ) {
                        let store = recovery_store_for_session(&session);
                        if let Err(error) = self.attach_pending_provider_root_claim(
                            &window_id,
                            recovery_id,
                            claim_token,
                            &store,
                            chrono::Duration::minutes(STARTUP_RECOVERY_LEASE_TTL_MINUTES),
                        ) {
                            tracing::warn!(
                                recovery_id,
                                window_id,
                                error = %error,
                                "startup provider-root claim attachment failed closed"
                            );
                            return Vec::new();
                        }
                    }
                }
                events
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session.id,
                    error = %error,
                    "failed to spawn restored agent window"
                );
                if let (Some(claim_token), Some(recovery_id)) = (
                    provider_root_claim_token.as_deref(),
                    session.recovery_id.as_deref(),
                ) {
                    let store = recovery_store_for_session(&session);
                    if let Err(release_error) = store.release_provider_root_claim_for_recovery(
                        recovery_id,
                        claim_token,
                        chrono::Utc::now(),
                    ) {
                        tracing::warn!(
                            recovery_id,
                            error = %release_error,
                            "failed to release startup provider-root claim after spawn failure"
                        );
                    }
                }
                if let Err(attention_error) =
                    mark_session_recovery_attention(&session, ATTENTION_LAUNCH_FAILED)
                {
                    tracing::warn!(
                        session_id = %session.id,
                        error = %attention_error,
                        "failed to mark recovery Attention after spawn failure"
                    );
                }
                Vec::new()
            }
        }
    }

    pub(super) fn attach_pending_provider_root_claim(
        &mut self,
        window_id: &str,
        recovery_id: &str,
        claim_token: &str,
        store: &gwt_core::recovery::RecoveryStore,
        claim_ttl: chrono::Duration,
    ) -> Result<(), String> {
        let pending = PendingProviderRootClaim {
            recovery_id: recovery_id.to_string(),
            claim_token: claim_token.to_string(),
            project_dir: store.root().to_path_buf(),
            claim_ttl,
        };
        self.pending_provider_root_claims
            .insert(window_id.to_string(), pending);
        let renewed_at = chrono::Utc::now();
        if let Err(error) = store.renew_provider_root_claim_for_recovery(
            recovery_id,
            claim_token,
            window_id,
            renewed_at,
            renewed_at + claim_ttl,
        ) {
            let reason = format!(
                "Provider-root claim could not be attached to the launched window: {error}"
            );
            self.abort_pending_provider_root_claim_attempt(window_id, &reason);
            return Err(reason);
        }
        self.schedule_pending_provider_root_claim_renewal(window_id);
        Ok(())
    }

    fn schedule_pending_provider_root_claim_renewal(&self, window_id: &str) {
        let Some(pending) = self.pending_provider_root_claims.get(window_id).cloned() else {
            return;
        };
        let window_id = window_id.to_string();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(StdDuration::from_secs(
                PROVIDER_ROOT_CLAIM_RENEW_INTERVAL_SECS,
            ));
            proxy.send(UserEvent::RenewProviderRootClaim {
                window_id,
                recovery_id: pending.recovery_id,
                claim_token: pending.claim_token.into(),
            });
        });
    }

    pub(crate) fn handle_pending_provider_root_claim_renewal(
        &mut self,
        window_id: String,
        recovery_id: String,
        claim_token: String,
    ) -> Vec<OutboundEvent> {
        let Some(pending) = self.pending_provider_root_claims.get(&window_id).cloned() else {
            return Vec::new();
        };
        if pending.recovery_id != recovery_id || pending.claim_token != claim_token {
            // A delayed timer from an older launch must never renew or abort a
            // newer claim that reused the same canvas window id.
            return Vec::new();
        }
        let renewed_at = chrono::Utc::now();
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&pending.project_dir);
        match store.renew_provider_root_claim_for_recovery(
            &pending.recovery_id,
            &pending.claim_token,
            &window_id,
            renewed_at,
            renewed_at + pending.claim_ttl,
        ) {
            Ok(_) => {
                self.schedule_pending_provider_root_claim_renewal(&window_id);
                Vec::new()
            }
            Err(error) => {
                tracing::error!(
                    recovery_id = %pending.recovery_id,
                    window_id,
                    error = %error,
                    "provider-root claim renewal failed; aborting exact recovery attempt"
                );
                let reason = format!("Provider-root claim ownership was lost: {error}");
                if !self.abort_pending_provider_root_claim_attempt(&window_id, &reason) {
                    return Vec::new();
                }
                let mut events = vec![self.workspace_state_broadcast()];
                events.extend(self.launch_next_startup_auto_resume_session());
                events
            }
        }
    }

    /// Fail closed after renewal/Ready CAS loss. The source recovery remains
    /// durable in Attention while only the replacement provider attempt is
    /// killed and removed.
    pub(super) fn abort_pending_provider_root_claim_attempt(
        &mut self,
        window_id: &str,
        reason: &str,
    ) -> bool {
        let pending = self.pending_provider_root_claims.get(window_id).cloned();
        let inferred_source = self
            .active_agent_sessions
            .get(window_id)
            .and_then(|active| {
                gwt_agent::Session::load_and_migrate(
                    &self
                        .sessions_dir
                        .join(format!("{}.toml", active.session_id)),
                )
                .ok()
            })
            .and_then(|target| {
                let target_recovery_id = target.recovery_id.as_deref()?;
                let handoff = target.recovery_continuation.as_ref()?;
                if target.session_mode != gwt_agent::SessionMode::Resume
                    || handoff.inherit_checkpoint
                    || handoff.target_recovery_id != target_recovery_id
                    || handoff.source_recovery_id.trim().is_empty()
                {
                    return None;
                }
                let project_root = target
                    .project_state_root
                    .as_deref()
                    .unwrap_or(&target.worktree_path);
                Some((
                    gwt_core::paths::gwt_project_dir_for_repo_path(project_root),
                    handoff.source_recovery_id.clone(),
                    (!handoff.source_session_id.trim().is_empty())
                        .then(|| handoff.source_session_id.clone()),
                ))
            });
        let source_session_id = self
            .pending_auto_resume_sources
            .get(window_id)
            .cloned()
            .or_else(|| {
                inferred_source
                    .as_ref()
                    .and_then(|(_, _, session_id)| session_id.clone())
            });
        let recovery_source = pending
            .as_ref()
            .map(|pending| (pending.project_dir.clone(), pending.recovery_id.clone()))
            .or_else(|| {
                inferred_source
                    .as_ref()
                    .map(|(project_dir, recovery_id, _)| (project_dir.clone(), recovery_id.clone()))
            });
        let pending = pending.or_else(|| {
            let (project_dir, recovery_id) = recovery_source.as_ref()?;
            let store = gwt_core::recovery::RecoveryStore::for_project_dir(project_dir);
            let record = store.load(recovery_id).ok().flatten()?;
            let root = record
                .provider_root
                .as_ref()
                .filter(|root| root.quality.is_authoritative())?;
            let claim = store
                .active_provider_root_claim(&record.provider, &root.root_id, chrono::Utc::now())
                .ok()
                .flatten()?;
            (claim.holder_recovery_id == *recovery_id).then_some(PendingProviderRootClaim {
                recovery_id: recovery_id.clone(),
                claim_token: claim.claim_token,
                project_dir: project_dir.clone(),
                claim_ttl: chrono::Duration::minutes(STARTUP_RECOVERY_LEASE_TTL_MINUTES),
            })
        });
        if recovery_source.is_none() && source_session_id.is_none() {
            return false;
        }

        let release_result = if self.pending_provider_root_claims.contains_key(window_id) {
            self.release_pending_provider_root_claim(window_id)
        } else if let Some(pending) = pending.as_ref() {
            let store = gwt_core::recovery::RecoveryStore::for_project_dir(&pending.project_dir);
            store
                .release_provider_root_claim_for_recovery(
                    &pending.recovery_id,
                    &pending.claim_token,
                    chrono::Utc::now(),
                )
                .map_err(|error| {
                    format!(
                        "release reconstructed provider-root claim for recovery {}: {error}",
                        pending.recovery_id
                    )
                })
        } else {
            Ok(false)
        };
        if let Err(error) = release_result {
            tracing::warn!(
                window_id,
                error = %error,
                "failed to release lost provider-root claim while aborting recovery"
            );
        }
        if let Some((project_dir, recovery_id)) = recovery_source.as_ref() {
            let store = gwt_core::recovery::RecoveryStore::for_project_dir(project_dir);
            match store.load(recovery_id) {
                Ok(Some(record))
                    if !matches!(
                        record.lifecycle,
                        gwt_core::recovery::RecoveryLifecycle::Resolved
                            | gwt_core::recovery::RecoveryLifecycle::Discarded
                    ) && (record.lifecycle
                        != gwt_core::recovery::RecoveryLifecycle::Attention
                        || record.lifecycle_reason.as_deref() != Some(reason)) =>
                {
                    if let Err(error) = store.set_lifecycle(
                        recovery_id,
                        gwt_core::recovery::RecoveryLifecycle::Attention,
                        Some(reason.to_string()),
                        format!("provider-claim-lost:{}:{}", recovery_id, record.generation),
                    ) {
                        tracing::warn!(
                            recovery_id,
                            error = %error,
                            "failed to preserve claim-lost recovery in Attention"
                        );
                    }
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    recovery_id,
                    error = %error,
                    "failed to load claim-lost recovery before Attention transition"
                ),
            }
        }
        if let Some(source_session_id) = source_session_id.as_deref() {
            let path = self.sessions_dir.join(format!("{source_session_id}.toml"));
            match gwt_agent::Session::load_and_migrate(&path) {
                Ok(session) => {
                    if let Err(error) = mark_session_recovery_attention(&session, reason) {
                        tracing::warn!(
                            source_session_id,
                            error = %error,
                            "failed to mark claim-lost source Session Attention"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    source_session_id,
                    error = %error,
                    "claim-lost source Session could not be loaded"
                ),
            }
        }

        self.stop_window_runtime_preserving_recovery(window_id);
        if let Some((project_dir, recovery_id)) = recovery_source.as_ref() {
            let store = gwt_core::recovery::RecoveryStore::for_project_dir(project_dir);
            if let Ok(Some(record)) = store.load(recovery_id) {
                if !matches!(
                    record.lifecycle,
                    gwt_core::recovery::RecoveryLifecycle::Resolved
                        | gwt_core::recovery::RecoveryLifecycle::Discarded
                ) {
                    let detail = reason.chars().take(280).collect::<String>();
                    let stopped_reason = format!(
                        "Recovery provider stopped before readiness after provider-root claim loss: {detail}"
                    );
                    if let Err(error) = store.set_lifecycle(
                        recovery_id,
                        gwt_core::recovery::RecoveryLifecycle::Attention,
                        Some(stopped_reason),
                        format!(
                            "provider-claim-lost-stopped:{}:{}",
                            recovery_id, record.generation
                        ),
                    ) {
                        tracing::warn!(
                            recovery_id,
                            error = %error,
                            "failed to persist provider-stop evidence after claim loss"
                        );
                    }
                }
            }
        }
        self.preserve_failed_auto_resume_attempt(window_id);
        self.pending_provider_root_claims.remove(window_id);
        self.pending_auto_resume_sources.remove(window_id);
        self.pending_workspace_resume_contexts.remove(window_id);
        self.pending_launch_feedback_contexts.remove(window_id);
        self.inflight_launches
            .retain(|_, (pending_window_id, _)| pending_window_id != window_id);
        if let Some(address) = self.window_lookup.get(window_id).cloned() {
            if let Some(tab) = self.tab_mut(&address.tab_id) {
                tab.workspace.close_window(&address.raw_id);
            }
        }
        self.window_lookup.remove(window_id);
        self.window_details.remove(window_id);
        self.launch_error_terminal_details.remove(window_id);
        let _ = self.persist();
        true
    }

    /// Mark the source recovery behind an in-flight automatic restore without
    /// clearing the source Session or its restore flag. Returns `true` when the
    /// window was an automatic recovery attempt, even if the ledger itself
    /// could not be loaded; callers use that to fail closed around Intake
    /// cleanup.
    fn release_pending_provider_root_claim(&mut self, window_id: &str) -> Result<bool, String> {
        let Some(pending) = self.pending_provider_root_claims.get(window_id).cloned() else {
            return Ok(false);
        };
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&pending.project_dir);
        let released = store
            .release_provider_root_claim_for_recovery(
                &pending.recovery_id,
                &pending.claim_token,
                chrono::Utc::now(),
            )
            .map_err(|error| {
                format!(
                    "release provider-root claim for recovery {}: {error}",
                    pending.recovery_id
                )
            })?;
        // `false` means this token is already stale/released. Either outcome
        // proves this runtime no longer owns the project claim.
        self.pending_provider_root_claims.remove(window_id);
        Ok(released)
    }

    pub(super) fn mark_pending_auto_resume_attention(
        &mut self,
        window_id: &str,
        reason: &str,
    ) -> bool {
        let Some(source_session_id) = self.pending_auto_resume_sources.get(window_id) else {
            return false;
        };
        let source_session_id = source_session_id.clone();
        if let Err(error) = self.release_pending_provider_root_claim(window_id) {
            tracing::warn!(
                window_id,
                error = %error,
                "failed to release provider-root claim before Attention transition"
            );
        }
        let path = self.sessions_dir.join(format!("{source_session_id}.toml"));
        match gwt_agent::Session::load_and_migrate(&path) {
            Ok(session) => {
                if let Err(error) = mark_session_recovery_attention(&session, reason) {
                    tracing::warn!(
                        source_session_id,
                        error = %error,
                        "failed to mark automatic recovery Attention"
                    );
                }
            }
            Err(error) => tracing::warn!(
                source_session_id,
                path = %path.display(),
                error = %error,
                "automatic recovery source Session could not be loaded"
            ),
        }
        true
    }

    /// Tear down only the failed provider attempt. This deliberately bypasses
    /// `mark_agent_session_stopped`, whose normal Intake semantics remove a
    /// clean detached worktree. The source Session in
    /// `pending_auto_resume_sources` remains active until a later provider
    /// Ready barrier succeeds.
    pub(super) fn preserve_failed_auto_resume_attempt(&mut self, window_id: &str) {
        if let Err(error) = self.release_pending_provider_root_claim(window_id) {
            tracing::warn!(
                window_id,
                error = %error,
                "failed to release provider-root claim while preserving failed recovery"
            );
        }
        let source_session_id = self.pending_auto_resume_sources.get(window_id).cloned();
        if let Some(attempt) = self.active_agent_sessions.remove(window_id) {
            if source_session_id.as_deref() != Some(attempt.session_id.as_str()) {
                let _ =
                    gwt_agent::update_session(&self.sessions_dir, &attempt.session_id, |session| {
                        session.update_status(gwt_agent::AgentStatus::Stopped);
                        session.restore_window_on_startup = false;
                        session.startup_restore_intent_recorded = true;
                        Ok(())
                    });
            }
        }
        self.runtimes.remove(window_id);
        self.deregister_pty_writer(window_id);
        self.recoverable_agent_error_windows.remove(window_id);
    }

    /// Replace a definitively rejected exact-resume provider with a fresh
    /// provider root carrying the bounded durable checkpoint. `None` means the
    /// window is not an automatic recovery and the historical diagnostic-only
    /// behavior should run. `Some` means recovery ownership was found and the
    /// source has either been relaunched or safely retained in Attention.
    pub(super) fn fallback_after_exact_resume_rejection(
        &mut self,
        window_id: &str,
    ) -> Option<Vec<OutboundEvent>> {
        let source_session_id = self.pending_auto_resume_sources.get(window_id)?.clone();
        let source_path = self.sessions_dir.join(format!("{source_session_id}.toml"));
        let source = match gwt_agent::Session::load_and_migrate(&source_path) {
            Ok(source) => source,
            Err(error) => {
                tracing::warn!(
                    source_session_id,
                    error = %error,
                    "exact-resume fallback source Session could not be loaded"
                );
                self.preserve_failed_auto_resume_attempt(window_id);
                return Some(Vec::new());
            }
        };
        let failed_target_session_id = match self.active_agent_sessions.get(window_id) {
            Some(attempt) => attempt.session_id.clone(),
            None => {
                let _ = mark_session_recovery_attention(&source, ATTENTION_LAUNCH_FAILED);
                self.preserve_failed_auto_resume_attempt(window_id);
                return Some(Vec::new());
            }
        };
        let failed_target_path = self
            .sessions_dir
            .join(format!("{failed_target_session_id}.toml"));
        let failed_target = match gwt_agent::Session::load_and_migrate(&failed_target_path) {
            Ok(target) => target,
            Err(error) => {
                tracing::warn!(
                    source_session_id,
                    failed_target_session_id,
                    error = %error,
                    "rejected exact-resume target Session could not be loaded"
                );
                let _ = mark_session_recovery_attention(&source, ATTENTION_MISSING_CONTEXT);
                self.preserve_failed_auto_resume_attempt(window_id);
                return Some(Vec::new());
            }
        };
        let mut config = match prepare_checkpoint_continuation_after_exact_rejection(&failed_target)
        {
            Ok(Some(config)) => config,
            Ok(None) => {
                let _ = mark_session_recovery_attention(&source, ATTENTION_MISSING_CONTEXT);
                self.preserve_failed_auto_resume_attempt(window_id);
                return Some(Vec::new());
            }
            Err(error) => {
                tracing::warn!(
                    source_session_id,
                    error = %error,
                    "exact-resume checkpoint fallback is unavailable"
                );
                let _ = mark_session_recovery_attention(&source, ATTENTION_MISSING_CONTEXT);
                self.preserve_failed_auto_resume_attempt(window_id);
                return Some(Vec::new());
            }
        };
        let failed_target_recovery_id = failed_target
            .recovery_id
            .as_deref()
            .expect("exact-rejection fallback requires a recovery identity");
        let failed_target_store = recovery_store_for_session(&failed_target);
        let failed_target_record = match failed_target_store.load(failed_target_recovery_id) {
            Ok(Some(record)) => record,
            Ok(None) => {
                let _ = mark_session_recovery_attention(&source, ATTENTION_MISSING_CONTEXT);
                self.preserve_failed_auto_resume_attempt(window_id);
                return Some(Vec::new());
            }
            Err(error) => {
                tracing::warn!(
                    source_session_id,
                    failed_target_session_id,
                    error = %error,
                    "rejected exact-resume Recovery Record could not be loaded"
                );
                let _ = mark_session_recovery_attention(&source, ATTENTION_MISSING_CONTEXT);
                self.preserve_failed_auto_resume_attempt(window_id);
                return Some(Vec::new());
            }
        };
        if let Err(error) = failed_target_store.interrupt_after_supervisor_stop(
            failed_target_recovery_id,
            failed_target_record.generation,
            &failed_target.id,
            chrono::Utc::now(),
            CONTINUATION_REASON_DEFINITIVE_REJECTION,
            format!(
                "exact-resume-rejected-v1:{}:{}",
                failed_target.id, failed_target_record.generation
            ),
        ) {
            tracing::warn!(
                source_session_id,
                failed_target_session_id,
                error = %error,
                "failed to persist definitive exact-resume rejection"
            );
            let _ = mark_session_recovery_attention(&source, ATTENTION_LAUNCH_FAILED);
            self.preserve_failed_auto_resume_attempt(window_id);
            return Some(Vec::new());
        }
        if let Err(error) = prepare_startup_successor(
            &failed_target,
            &mut config,
            CONTINUATION_REASON_DEFINITIVE_REJECTION,
            true,
        ) {
            tracing::warn!(
                source_session_id,
                failed_target_session_id,
                error = %error,
                "exact-resume checkpoint fallback successor could not be prepared"
            );
            let _ = mark_session_recovery_attention(&source, ATTENTION_MISSING_CONTEXT);
            self.preserve_failed_auto_resume_attempt(window_id);
            return Some(Vec::new());
        }
        let Some(address) = self.window_lookup.get(window_id).cloned() else {
            let _ = mark_session_recovery_attention(&source, ATTENTION_LAUNCH_FAILED);
            self.preserve_failed_auto_resume_attempt(window_id);
            return Some(Vec::new());
        };
        let Some(geometry) = self
            .tab(&address.tab_id)
            .and_then(|tab| tab.workspace.window(&address.raw_id))
            .map(|window| window.geometry.clone())
        else {
            let _ = mark_session_recovery_attention(&source, ATTENTION_LAUNCH_FAILED);
            self.preserve_failed_auto_resume_attempt(window_id);
            return Some(Vec::new());
        };
        let tab_id = address.tab_id.clone();

        self.preserve_failed_auto_resume_attempt(window_id);
        self.pending_auto_resume_sources.remove(window_id);
        self.pending_workspace_resume_contexts.remove(window_id);
        self.pending_launch_feedback_contexts.remove(window_id);
        self.inflight_launches
            .retain(|_, (pending_window_id, _)| pending_window_id != window_id);
        if let Some(tab) = self.tab_mut(&tab_id) {
            tab.workspace.close_window(&address.raw_id);
        }
        self.window_lookup.remove(window_id);
        self.window_details.remove(window_id);
        self.launch_error_terminal_details.remove(window_id);

        let workspace_resume_context = Some(workspace_resume_context_for_work_item(
            &failed_target.worktree_path,
            Some(failed_target.branch.as_str()),
            &failed_target.worktree_path,
        ));
        let events = self.spawn_restored_agent_session_with_config(
            &tab_id,
            failed_target.clone(),
            config,
            workspace_resume_context,
            geometry,
            None,
        );
        if !self
            .pending_auto_resume_sources
            .values()
            .any(|candidate| candidate == &failed_target_session_id)
        {
            let _ = mark_session_recovery_attention(&source, ATTENTION_LAUNCH_FAILED);
        }
        Some(events)
    }

    pub(super) fn recovery_provider_stopped_attention_reason() -> &'static str {
        ATTENTION_PROVIDER_STOPPED
    }

    /// SPEC-2356 安心 Addendum (FR-044): relaunch a stopped/errored `Agent`
    /// window in place. Reuses the same persisted-Session resume primitive the
    /// startup window restore uses ([`Self::spawn_restored_agent_session`]),
    /// which removes the paused placeholder and re-spawns the agent into the
    /// reused window id, preserving the window and appending to its prior
    /// output. Returns an empty event list when the window has no resumable
    /// Session (e.g. a never-launched placeholder) so the kill-switch UI can
    /// surface "nothing to restart" instead of spawning a blank agent.
    pub(crate) fn restart_agent_window_in_place(
        &mut self,
        tab_id: &str,
        raw_id: &str,
        fallback_geometry: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(session_id) = self
            .tab(tab_id)
            .and_then(|tab| tab.workspace.window(raw_id))
            .and_then(|window| window.session_id.clone())
        else {
            return Vec::new();
        };
        let path = self.sessions_dir.join(format!("{session_id}.toml"));
        let Ok(session) = gwt_agent::Session::load_and_migrate(&path) else {
            return Vec::new();
        };
        let workspace_resume_context = Some(workspace_resume_context_for_work_item(
            &session.worktree_path,
            Some(session.branch.as_str()),
            &session.worktree_path,
        ));
        let mut events = vec![self.workspace_state_broadcast()];
        events.append(&mut self.spawn_restored_agent_session(
            tab_id,
            session,
            workspace_resume_context,
            fallback_geometry,
        ));
        events
    }

    /// Restore every process window the user did not explicitly close in a
    /// freshly opened/restored project tab (Issue #2942). Closing a window
    /// removes it from the persisted workspace, so the persisted process
    /// windows are exactly the set to restart: agents resume via their native
    /// session id (or launch fresh when none exists), and non-agent process
    /// windows (e.g. Shell) launch fresh. Runs synchronously because each
    /// placeholder already carries its geometry, so no frontend canvas bounds
    /// round-trip is required. The startup `bootstrap` queue only covers tabs
    /// open at launch, so projects opened via Open Project / Reopen Recent were
    /// never restored before this path existed.
    pub(super) fn restore_open_project_windows(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        let windows = match self.tab(tab_id) {
            Some(tab) if tab.kind == gwt::ProjectKind::Git && !tab.migration_pending => tab
                .workspace
                .persisted()
                .windows
                .iter()
                .filter(|window| {
                    window.preset.requires_process()
                        && window.status == WindowProcessStatus::Stopped
                })
                .cloned()
                .collect::<Vec<_>>(),
            _ => return Vec::new(),
        };

        let mut events = Vec::new();
        for window in windows {
            let combined = combined_window_id(tab_id, &window.id);
            // A window with a live PTY/runtime is already running (e.g. when an
            // already-open project tab is re-selected); only paused placeholders
            // should be restarted. `window_lookup` is the registry of known
            // windows, not the set of running ones, so it must not gate here.
            if self.runtimes.contains_key(&combined) {
                continue;
            }
            if crate::runtime_support::window_is_agent_pane(&window) {
                let Some(session_id) = window.session_id.clone() else {
                    continue;
                };
                let path = self.sessions_dir.join(format!("{session_id}.toml"));
                let Ok(session) = gwt_agent::Session::load_and_migrate(&path) else {
                    continue;
                };
                if !session.worktree_path.exists() {
                    continue;
                }
                if self
                    .active_agent_sessions
                    .values()
                    .any(|active| active.session_id == session.id)
                {
                    continue;
                }
                let workspace_resume_context = Some(workspace_resume_context_for_work_item(
                    &session.worktree_path,
                    Some(session.branch.as_str()),
                    &session.worktree_path,
                ));
                let fallback_geometry = window.geometry.clone();
                let mut spawned = self.spawn_restored_agent_session(
                    tab_id,
                    session,
                    workspace_resume_context,
                    fallback_geometry,
                );
                events.append(&mut spawned);
            } else {
                events.extend(self.start_window(
                    tab_id,
                    &window.id,
                    window.preset,
                    window.geometry.clone(),
                ));
            }
        }
        events
    }

    /// Find the tab holding a persisted, paused (`Stopped`) agent placeholder
    /// window backed by `session_id`. Its presence proves the user did not
    /// explicitly close that window (Issue #2942), so the session must restore
    /// regardless of status drift or age.
    fn paused_placeholder_tab_for_session(&self, session: &gwt_agent::Session) -> Option<String> {
        if session.startup_restore_intent_recorded && !session.restore_window_on_startup {
            return None;
        }
        let session_id = session.id.as_str();
        self.tabs
            .iter()
            .filter(|tab| tab.kind == gwt::ProjectKind::Git && !tab.migration_pending)
            .find(|tab| {
                tab.workspace.persisted().windows.iter().any(|window| {
                    window.status == WindowProcessStatus::Stopped
                        && crate::runtime_support::window_is_agent_pane(window)
                        && window.session_id.as_deref() == Some(session_id)
                })
            })
            .map(|tab| tab.id.clone())
    }

    fn remove_stale_paused_agent_window(
        &mut self,
        tab_id: &str,
        session_id: &str,
    ) -> Option<WindowGeometry> {
        let tab = self.tab_mut(tab_id)?;
        // SPEC-1921 Phase 65 (T337): stale placeholder removal must cover the
        // full Agent-family preset set (`Agent`, `Claude`, `Codex`), not just
        // the legacy `Agent` preset — otherwise a resumed Claude/Codex window
        // spawns next to its surviving placeholder and loses the restored
        // geometry.
        let stale = tab
            .workspace
            .persisted()
            .windows
            .iter()
            .find(|w| {
                crate::runtime_support::window_is_agent_pane(w)
                    && w.status == WindowProcessStatus::Stopped
                    && w.session_id.as_deref() == Some(session_id)
            })
            .map(|w| (w.id.clone(), w.geometry.clone()));
        let (raw_id, geometry) = stale?;
        tab.workspace.close_window(&raw_id);
        let combined = combined_window_id(tab_id, &raw_id);
        self.window_lookup.remove(&combined);
        self.window_details.remove(&combined);
        Some(geometry)
    }

    fn auto_resume_tab_id_for_session(&self, session: &gwt_agent::Session) -> Option<String> {
        if let Some(tab) = self.tabs.iter().find(|tab| {
            tab.kind == gwt::ProjectKind::Git
                && !tab.migration_pending
                && same_worktree_path(&tab.project_root, &session.worktree_path)
        }) {
            return Some(tab.id.clone());
        }

        // Issue #2942: a session's worktree belongs to the tab whose project
        // shares the same main worktree root (the gwt workspace home / bare
        // layout root). `repo_hash` / `project_scope_hash` differ between a
        // workspace-home project_root and its linked worktrees, so scope-hash
        // equality alone fails to associate worktree-backed agent sessions with
        // the parent tab and they never auto-resume on startup.
        if let Ok(session_root) = gwt_git::worktree::main_worktree_root(&session.worktree_path) {
            if let Some(tab) = self.tabs.iter().find(|tab| {
                tab.kind == gwt::ProjectKind::Git
                    && !tab.migration_pending
                    && same_worktree_path(&tab.main_worktree_root(), &session_root)
            }) {
                return Some(tab.id.clone());
            }
        }

        let session_scope = session_project_scope_hash(session)?;
        self.tabs
            .iter()
            .find(|tab| {
                tab.kind == gwt::ProjectKind::Git
                    && !tab.migration_pending
                    && gwt_core::paths::project_scope_hash(&tab.project_root).to_string()
                        == session_scope
            })
            .map(|tab| tab.id.clone())
    }

    fn load_recovery_sessions(&self) -> std::io::Result<Vec<gwt_agent::Session>> {
        let paths = gwt_agent::discover_session_toml_paths(&self.sessions_dir)?;
        let mut budget = gwt_agent::SessionInventoryReadBudget::default();
        paths
            .into_iter()
            .map(|path| {
                let session_id = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::trim)
                    .filter(|session_id| !session_id.is_empty())
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Session TOML has no valid UTF-8 identity: {}",
                                path.display()
                            ),
                        )
                    })?;
                gwt_agent::update_session_with_inventory_budget(
                    &self.sessions_dir,
                    session_id,
                    &mut budget,
                    |session| {
                        if session.worktree_path.exists()
                            && session.should_mark_interrupted_from_lifecycle()
                        {
                            session.update_status(gwt_agent::AgentStatus::Interrupted);
                        }
                        Ok(())
                    },
                )
            })
            .collect()
    }

    pub(crate) fn set_agent_capability_issuer(&mut self, issuer: AgentCapabilityIssuer) {
        self.agent_capability_issuer = Some(issuer);
    }

    /// SPEC-2785 FR-E: capture the embedded server URL after the axum bind
    /// completes so `open_server_url_events` can reject mismatched origin
    /// requests before invoking the OS opener.
    pub(crate) fn set_server_url(&mut self, url: String) {
        self.server_url = Some(url);
    }

    /// SPEC-2970: wire the usage poller's refresh handle so frontend toggles
    /// can request an immediate re-poll.
    pub(crate) fn set_usage_refresh(&mut self, refresh: std::sync::Arc<tokio::sync::Notify>) {
        self.usage_refresh = Some(refresh);
    }
}

#[cfg(test)]
mod recovery_spawn_boundary_tests {
    use super::*;
    use chrono::{Duration, Utc};
    use gwt_core::test_support::ScopedEnvVar;

    #[test]
    fn incomplete_session_inventory_keeps_unscanned_intake_out_of_prune_path() {
        let temp = tempfile::tempdir().unwrap();
        let unscanned = temp.path().join(".intake-unscanned");
        std::fs::create_dir(&unscanned).unwrap();
        let session_paths = HashSet::new();

        let protected =
            combine_startup_recovery_protection(false, &session_paths, Some(HashSet::new()));
        if protected.is_some() {
            // Mirrors bootstrap's `let Some(protected_paths) = ... else {
            // continue; }` boundary. An incomplete Session inventory never
            // enters the destructive orphan-prune branch.
            std::fs::remove_dir_all(&unscanned).unwrap();
        }

        assert!(unscanned.exists());
    }

    #[test]
    fn startup_quarantines_spawn_requested_instead_of_duplicate_launching() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        let worktree = repo.join(".intake-1");
        std::fs::create_dir_all(&worktree).unwrap();
        let mut session = gwt_agent::Session::new(&worktree, "", gwt_agent::AgentId::Codex);
        session.project_state_root = Some(repo.clone());
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.is_ephemeral = true;
        session.recovery_launch_stage =
            Some(gwt_agent::session::RecoveryLaunchStage::SpawnRequested);
        let recovery_id = session.recovery_id.clone().unwrap();
        let store = recovery_store_for_session(&session);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: recovery_id.clone(),
                    session_id: session.id.clone(),
                    repo_id: gwt_core::paths::project_scope_hash(&repo).to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree,
                    launch_base_ref: Some("develop".to_string()),
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "1".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Investigate".to_string(),
                    created_at: session.created_at,
                },
                "create-spawn-boundary",
            )
            .unwrap();
        store
            .advance_launch_stage(
                &recovery_id,
                gwt_core::recovery::RecoveryLaunchStage::SpawnRequested,
                None,
                "spawn-requested",
            )
            .unwrap();

        assert!(quarantine_indeterminate_spawn(&session).unwrap());
        let quarantined = store.load(&recovery_id).unwrap().unwrap();
        assert_eq!(
            quarantined.lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Attention
        );
        assert_eq!(
            quarantined.launch_stage,
            gwt_core::recovery::RecoveryLaunchStage::SpawnRequested
        );
        assert!(quarantine_indeterminate_spawn(&session).unwrap());
    }

    #[test]
    fn cold_start_proves_supervisor_stop_for_every_known_spawned_stage() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

        for (index, stage) in [
            gwt_core::recovery::RecoveryLaunchStage::ProcessSpawned,
            gwt_core::recovery::RecoveryLaunchStage::ProviderBound,
        ]
        .into_iter()
        .enumerate()
        {
            let repo = temp.path().join(format!("repo-known-spawn-{index}"));
            let worktree = repo.join(format!(".intake-{}", index + 2));
            std::fs::create_dir_all(&worktree).unwrap();
            let mut session = gwt_agent::Session::new(&worktree, "", gwt_agent::AgentId::Codex);
            session.id = format!("known-spawn-session-{index}");
            session.project_state_root = Some(repo.clone());
            session.session_kind = Some(gwt_skills::SessionKind::Intake);
            session.is_ephemeral = true;
            session.status = gwt_agent::AgentStatus::Running;
            let recovery_id = session.recovery_id.clone().unwrap();
            let store = recovery_store_for_session(&session);
            store
                .create(
                    gwt_core::recovery::CreateRecovery {
                        recovery_id: recovery_id.clone(),
                        session_id: session.id.clone(),
                        repo_id: gwt_core::paths::project_scope_hash(&repo).to_string(),
                        session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                        worktree_path: worktree,
                        launch_base_ref: Some("develop".to_string()),
                        launch_base_oid: "1".repeat(40),
                        launch_head_oid: "1".repeat(40),
                        provider: "codex".to_string(),
                        model: None,
                        runtime: "host".to_string(),
                        initial_prompt: "Investigate".to_string(),
                        created_at: session.created_at,
                    },
                    format!("create-known-spawn-{index}"),
                )
                .unwrap();
            if stage == gwt_core::recovery::RecoveryLaunchStage::ProviderBound {
                store
                    .bind_root_semantic(
                        &recovery_id,
                        &format!("known-spawn-root-{index}"),
                        None,
                        gwt_core::recovery::BindingQuality::Verified,
                        format!("bind-known-spawn-{index}"),
                    )
                    .unwrap();
            } else {
                store
                    .advance_launch_stage(
                        &recovery_id,
                        stage,
                        None,
                        format!("advance-known-spawn-{index}"),
                    )
                    .unwrap();
            }

            assert!(!reconcile_stopped_session_recovery(&session, false).unwrap());
            assert!(store
                .load(&recovery_id)
                .unwrap()
                .unwrap()
                .supervisor_stop_proof
                .is_none());

            assert!(reconcile_stopped_session_recovery(&session, true).unwrap());
            let interrupted = store.load(&recovery_id).unwrap().unwrap();
            assert_eq!(
                interrupted.lifecycle,
                gwt_core::recovery::RecoveryLifecycle::Interrupted
            );
            assert_eq!(interrupted.launch_stage, stage);
            assert_eq!(
                interrupted
                    .supervisor_stop_proof
                    .as_ref()
                    .map(|proof| proof.session_id.as_str()),
                Some(session.id.as_str())
            );
        }
    }

    #[test]
    fn cold_start_proves_supervisor_stop_for_ready_execution() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo-ready-execution");
        let worktree = repo.join("work-execution");
        std::fs::create_dir_all(&worktree).unwrap();
        let mut session =
            gwt_agent::Session::new(&worktree, "work/execution", gwt_agent::AgentId::Codex);
        session.id = "cold-ready-execution-session".to_string();
        session.project_state_root = Some(repo.clone());
        session.session_kind = Some(gwt_skills::SessionKind::Execution);
        session.is_ephemeral = false;
        session.status = gwt_agent::AgentStatus::Running;
        session.restore_window_on_startup = true;
        session.agent_session_id = Some("cold-ready-execution-root".to_string());
        let recovery_id = session.recovery_id.clone().unwrap();
        let store = recovery_store_for_session(&session);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: recovery_id.clone(),
                    session_id: session.id.clone(),
                    repo_id: gwt_core::paths::project_scope_hash(&repo).to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Execution,
                    worktree_path: worktree,
                    launch_base_ref: Some("develop".to_string()),
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "1".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Continue execution".to_string(),
                    created_at: session.created_at,
                },
                "create-cold-ready-execution",
            )
            .unwrap();
        store
            .bind_root_semantic(
                &recovery_id,
                session.agent_session_id.as_deref().unwrap(),
                None,
                gwt_core::recovery::BindingQuality::Verified,
                "bind-cold-ready-execution",
            )
            .unwrap();
        store
            .complete_provider_ready(&recovery_id, Utc::now(), "ready-cold-execution")
            .unwrap();

        assert!(reconcile_stopped_session_recovery(&session, true).unwrap());
        let interrupted = store.load(&recovery_id).unwrap().unwrap();
        assert_eq!(
            interrupted.lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Interrupted
        );
        assert_eq!(
            interrupted.launch_stage,
            gwt_core::recovery::RecoveryLaunchStage::Ready
        );
        assert!(recovery_was_interrupted_after_supervisor_stop(
            &session,
            &interrupted
        ));
        assert!(startup_managed_recovery_resume_candidate(&session).unwrap());
    }

    #[test]
    fn cold_start_does_not_resolve_unclassified_attention_as_supervisor_stop() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo-unclassified-attention");
        let worktree = repo.join(".intake-7");
        std::fs::create_dir_all(&worktree).unwrap();
        let mut session = gwt_agent::Session::new(&worktree, "", gwt_agent::AgentId::Codex);
        session.id = "session-unclassified-attention".to_string();
        session.project_state_root = Some(repo.clone());
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.is_ephemeral = true;
        session.status = gwt_agent::AgentStatus::Running;
        let recovery_id = session.recovery_id.clone().unwrap();
        let store = recovery_store_for_session(&session);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: recovery_id.clone(),
                    session_id: session.id.clone(),
                    repo_id: gwt_core::paths::project_scope_hash(&repo).to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree,
                    launch_base_ref: Some("develop".to_string()),
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "1".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Investigate".to_string(),
                    created_at: session.created_at,
                },
                "create-unclassified-attention",
            )
            .unwrap();
        store
            .bind_root_semantic(
                &recovery_id,
                "ambiguous-provider-root",
                None,
                gwt_core::recovery::BindingQuality::Verified,
                "bind-unclassified-attention",
            )
            .unwrap();
        store
            .set_lifecycle(
                &recovery_id,
                gwt_core::recovery::RecoveryLifecycle::Attention,
                Some("legacy_import_attention:multiple_provider_roots".to_string()),
                "mark-unclassified-attention",
            )
            .unwrap();

        assert!(!reconcile_stopped_session_recovery(&session, true).unwrap());
        let unchanged = store.load(&recovery_id).unwrap().unwrap();
        assert_eq!(
            unchanged.lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Attention
        );
        assert!(unchanged.supervisor_stop_proof.is_none());
    }

    #[test]
    fn startup_interruption_clears_an_expired_unrenewed_recovery_lease() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        let worktree = repo.join(".intake-1");
        std::fs::create_dir_all(&worktree).unwrap();
        let mut session = gwt_agent::Session::new(&worktree, "", gwt_agent::AgentId::Codex);
        session.project_state_root = Some(repo.clone());
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.is_ephemeral = true;
        session.status = gwt_agent::AgentStatus::Interrupted;
        let recovery_id = session.recovery_id.clone().unwrap();
        let provider_root_id = "provider-root-startup-expired";
        let store = recovery_store_for_session(&session);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: recovery_id.clone(),
                    session_id: session.id.clone(),
                    repo_id: gwt_core::paths::project_scope_hash(&repo).to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree,
                    launch_base_ref: Some("develop".to_string()),
                    launch_base_oid: "1".repeat(40),
                    launch_head_oid: "1".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Investigate".to_string(),
                    created_at: session.created_at,
                },
                "create-startup-expired",
            )
            .unwrap();
        store
            .bind_root(
                &recovery_id,
                gwt_core::recovery::ProviderRootBinding {
                    root_id: provider_root_id.to_string(),
                    session_tree_id: None,
                    quality: gwt_core::recovery::BindingQuality::Verified,
                    bound_at: Utc::now(),
                },
                "bind-startup-expired-provider-root",
            )
            .unwrap();
        let claimed_at = Utc::now() - Duration::minutes(10);
        let record = store.load(&recovery_id).unwrap().unwrap();
        store
            .claim_recovery_with_provider_root(
                &recovery_id,
                record.generation,
                provider_root_id,
                false,
                gwt_core::recovery::RecoveryLease {
                    lease_id: "startup-expired-lease".to_string(),
                    holder_id: "startup-expired-holder".to_string(),
                    acquired_at: claimed_at,
                    expires_at: claimed_at + Duration::minutes(1),
                },
                "startup-expired-window",
                "startup expired integration test",
                "claim-startup-expired",
            )
            .unwrap();

        reconcile_stopped_session_recovery(&session, true).unwrap();

        let reconciled = store.load(&recovery_id).unwrap().unwrap();
        assert_eq!(
            reconciled.lifecycle,
            gwt_core::recovery::RecoveryLifecycle::Interrupted
        );
        assert!(reconciled.recovery_lease.is_none());
        assert!(store
            .active_provider_root_claim("codex", provider_root_id, Utc::now())
            .unwrap()
            .is_none());
    }

    #[test]
    fn cold_start_records_supervisor_stop_for_every_exact_resume_status() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());

        for (index, status) in [
            gwt_agent::AgentStatus::Running,
            gwt_agent::AgentStatus::Idle,
            gwt_agent::AgentStatus::WaitingInput,
            gwt_agent::AgentStatus::Interrupted,
        ]
        .into_iter()
        .enumerate()
        {
            let repo = temp.path().join(format!("repo-{index}"));
            let worktree = repo.join(format!(".intake-{index}"));
            std::fs::create_dir_all(&worktree).unwrap();
            let mut session = gwt_agent::Session::new(&worktree, "", gwt_agent::AgentId::Codex);
            session.id = format!("cold-ready-session-{index}");
            session.project_state_root = Some(repo.clone());
            session.session_kind = Some(gwt_skills::SessionKind::Intake);
            session.is_ephemeral = true;
            session.status = status;
            session.agent_session_id = Some(format!("cold-ready-root-{index}"));
            let recovery_id = session.recovery_id.clone().unwrap();
            let store = recovery_store_for_session(&session);
            store
                .create(
                    gwt_core::recovery::CreateRecovery {
                        recovery_id: recovery_id.clone(),
                        session_id: session.id.clone(),
                        repo_id: gwt_core::paths::project_scope_hash(&repo).to_string(),
                        session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                        worktree_path: worktree,
                        launch_base_ref: Some("develop".to_string()),
                        launch_base_oid: "1".repeat(40),
                        launch_head_oid: "1".repeat(40),
                        provider: "codex".to_string(),
                        model: None,
                        runtime: "host".to_string(),
                        initial_prompt: "Investigate".to_string(),
                        created_at: session.created_at,
                    },
                    format!("create-cold-ready-{index}"),
                )
                .unwrap();
            store
                .bind_root_semantic(
                    &recovery_id,
                    session.agent_session_id.as_deref().unwrap(),
                    None,
                    gwt_core::recovery::BindingQuality::Verified,
                    format!("bind-cold-ready-{index}"),
                )
                .unwrap();
            store
                .complete_provider_ready(&recovery_id, Utc::now(), format!("ready-cold-{index}"))
                .unwrap();

            assert!(reconcile_stopped_session_recovery(&session, true).unwrap());
            let interrupted = store.load(&recovery_id).unwrap().unwrap();
            assert_eq!(
                interrupted.lifecycle,
                gwt_core::recovery::RecoveryLifecycle::Interrupted,
                "status {status:?}"
            );
            assert_eq!(
                interrupted.launch_stage,
                gwt_core::recovery::RecoveryLaunchStage::Ready
            );
            assert_eq!(
                interrupted
                    .supervisor_stop_proof
                    .as_ref()
                    .map(|proof| proof.session_id.as_str()),
                Some(session.id.as_str())
            );
        }
    }
}
