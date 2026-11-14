//! Resident PM pane lifecycle (SPEC-3431): the GUI-side singleton gate.
//!
//! The durable half of the singleton (project-state `pm.json`) lives in
//! `gwt::pm_registry`; this module supplies the authoritative liveness
//! answer from the in-memory pane registry and drives the ensure flow:
//! live PM → focus, stale registration → resume the same conversation,
//! nothing registered → fresh silent spawn (branchless, explicit PM
//! worktree, `$gwt-pm` bootstrap prompt).
//!
//! The gwt-pm guidance skill that prompt resolves against is written by
//! `managed_assets::materialize_managed_gwt_assets_for_targets` — the single
//! writer, keyed on the canonical PM worktree path. Fresh spawn, resume,
//! crash respawn, and every later refresh funnel through it, so this module
//! never materializes the skill itself.
//!
//! Fresh spawns read the project's own `PmSettings::launch_profile` and fall
//! back to a fixed default. They deliberately avoid the Launch Wizard profile
//! machinery: those profiles are derived from unrelated launches and do not
//! exist at all on a fresh project (the Issue Monitor bootstrap trap), and the
//! PM must come up regardless.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use gwt::persistence::{WindowGeometry, WindowProcessStatus};
use gwt::pm_registry::{self, PmLaunchProfile, PmRegistration};
use gwt::{PmAgentOption, WindowPreset};
use uuid::Uuid;

use super::{
    AgentCapabilityGrant, AgentCapabilityIssuer, AgentPmSendResponder, AppRuntime, BackendEvent,
    BlockingTaskSpawner, ClientId, OutboundEvent, PmSubmitReservationRegistry, PtyWriterRegistry,
    UserEvent,
};

/// Fixed geometry for a freshly spawned PM pane.
const PM_WINDOW_GEOMETRY: WindowGeometry = WindowGeometry {
    x: 96.0,
    y: 96.0,
    width: 860.0,
    height: 520.0,
};

/// Bootstrap prompt: invokes the materialized gwt-pm guidance skill.
const PM_BOOTSTRAP_PROMPT: &str = "$gwt-pm";

/// SPEC-3431 T-093 (FR-012): a wake the monitor-event path decided on — which
/// pane receives the prompt and what it says. The window id is only ever the
/// registered PM session's live pane, resolved inside the decision; no caller
/// can point the wake anywhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PmWakeDecision {
    pub(crate) window_id: String,
    pub(crate) prompt: String,
    project_root: PathBuf,
    registration: PmRegistration,
    operation_id: String,
    body_sha256: String,
    kind: PmWakeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PmWakeKind {
    Delta { signals: BTreeSet<String> },
    Periodic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmWakeDeliveryPhase {
    InFlight,
    NeedsRecovery,
    AwaitingCommitRetry,
}

#[derive(Clone)]
pub(crate) struct PendingPmWakeDelivery {
    decision: PmWakeDecision,
    phase: PmWakeDeliveryPhase,
    expected_pty: Option<Arc<gwt_terminal::PtyHandle>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PmWakeDeliveryResult {
    Verified,
    SubmitUnverified,
    CommitRetryable { error: String },
    Terminal { error: String },
}

const PM_WAKE_SUBMIT_ATTEMPTS: usize = 2;

#[cfg(test)]
type PmPaneAfterBodyTestHook = Box<dyn FnOnce() + Send + 'static>;

#[cfg(test)]
static PM_PANE_AFTER_BODY_TEST_HOOK: std::sync::LazyLock<
    std::sync::Mutex<Option<PmPaneAfterBodyTestHook>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

#[cfg(test)]
pub(crate) fn set_pm_pane_after_body_hook_for_test(hook: impl FnOnce() + Send + 'static) {
    let mut slot = PM_PANE_AFTER_BODY_TEST_HOOK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(slot.replace(Box::new(hook)).is_none());
}

#[cfg(test)]
pub(crate) fn clear_pm_pane_after_body_hook_for_test() {
    PM_PANE_AFTER_BODY_TEST_HOOK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
}

#[cfg(test)]
fn invoke_pm_pane_after_body_test_hook() {
    let hook = PM_PANE_AFTER_BODY_TEST_HOOK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(not(test))]
fn invoke_pm_pane_after_body_test_hook() {}

fn protected_pm_wake_prompt(body: String) -> (String, String, String) {
    let operation_id = Uuid::new_v4().hyphenated().to_string();
    let body_sha256 = pm_registry::pm_delivery_prompt_sha256(&body);
    let prompt = protected_delivery_prompt(&operation_id, &body);
    (operation_id, prompt, body_sha256)
}

fn protected_delivery_prompt(operation_id: &str, body: &str) -> String {
    let body_sha256 = pm_registry::pm_delivery_prompt_sha256(body);
    format!("{body} [gwt-delivery:{operation_id}:{body_sha256}]\r")
}

fn canonical_pm_message_text(text: &str) -> Result<String, String> {
    let body = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\r'))
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    if body.is_empty() {
        return Err("pm pane send: message must not be empty".to_string());
    }
    if body.len() > 8_192 {
        return Err("pm pane send: message exceeds 8192 bytes".to_string());
    }
    if body.chars().any(char::is_control) {
        return Err("pm pane send: message must be one line without control bytes".to_string());
    }
    Ok(format!("{body}\r"))
}

fn current_pty_writer(
    writers: &PtyWriterRegistry,
    window_id: &str,
) -> Result<Arc<gwt_terminal::PtyHandle>, String> {
    writers
        .read()
        .map_err(|_| "PM delivery PTY registry is unavailable".to_string())?
        .get(window_id)
        .cloned()
        .ok_or_else(|| format!("no live runtime for pane {window_id}"))
}

fn prepare_pm_send_context(
    prefs_path: &Path,
    principal_session_id: &str,
    principal_session_is_live: bool,
    writers: &PtyWriterRegistry,
    window_id: &str,
) -> Result<(PmRegistration, Arc<gwt_terminal::PtyHandle>), String> {
    let registration = pm_registry::load_pm_registration_locked(prefs_path)
        .map_err(|error| format!("pm pane send: pm registration unreadable: {error}"))?
        .ok_or_else(|| {
            "pm pane send: no registered PM for this project (FR-111 requires the live PM principal)"
                .to_string()
        })?;
    if registration.session_id != principal_session_id {
        return Err(
            "pm pane send: authenticated session is not the registered PM (FR-111 refuses foreign principals)"
                .to_string(),
        );
    }
    if !principal_session_is_live {
        return Err(
            "pm pane send: the registered PM has no live pane (stale registration; FR-111 refuses)"
                .to_string(),
        );
    }
    let expected_pty = current_pty_writer(writers, window_id)?;
    Ok((registration, expected_pty))
}

fn reserve_pm_submit(
    reservations: &PmSubmitReservationRegistry,
    window_id: &str,
    operation_id: &str,
) -> Result<bool, String> {
    let mut reservations = reservations
        .lock()
        .map_err(|_| "PM submit reservation registry is unavailable".to_string())?;
    match reservations.get(window_id) {
        Some(existing) if existing == operation_id => Ok(true),
        Some(_) => Ok(false),
        None => {
            reservations.insert(window_id.to_string(), operation_id.to_string());
            Ok(true)
        }
    }
}

fn release_pm_submit(
    reservations: &PmSubmitReservationRegistry,
    window_id: &str,
    operation_id: &str,
) {
    if let Ok(mut reservations) = reservations.lock() {
        if reservations.get(window_id).map(String::as_str) == Some(operation_id) {
            reservations.remove(window_id);
        }
    }
}

enum PmDeliveryReplayOutcome {
    Missing,
    Verified,
    Refused(String),
}

/// Reconcile one caller-stable operation without consulting the current pane
/// generation. A durable terminal receipt remains authoritative after the
/// target pane has stopped or been replaced, while an open Prepared row is
/// conservatively closed as Ambiguous and never replays the body.
fn reconcile_pm_delivery_replay(
    receipt_path: &Path,
    operation_id: &str,
    principal_session_id: &str,
    target_window_id: &str,
    body_sha256: &str,
) -> Result<PmDeliveryReplayOutcome, String> {
    let receipts = pm_registry::load_pm_delivery_receipts(receipt_path)
        .map_err(|error| format!("PM delivery receipt lookup failed: {error}"))?;
    let Some(first) = receipts
        .iter()
        .find(|receipt| receipt.operation_id == operation_id)
    else {
        return Ok(PmDeliveryReplayOutcome::Missing);
    };
    if first.principal_session_id != principal_session_id
        || first.target_window_id != target_window_id
        || first.body_sha256 != body_sha256
    {
        return Ok(PmDeliveryReplayOutcome::Refused(
            "pm pane send: operation identity conflicts with its durable receipt".to_string(),
        ));
    }
    if let Some(terminal) = receipts.iter().rev().find(|receipt| {
        receipt.operation_id == operation_id
            && receipt.status != pm_registry::PmDeliveryReceiptStatus::Prepared
    }) {
        return Ok(match terminal.status {
            pm_registry::PmDeliveryReceiptStatus::Verified => PmDeliveryReplayOutcome::Verified,
            status => PmDeliveryReplayOutcome::Refused(format!(
                "pm pane send: operation is already terminal: {status:?}"
            )),
        });
    }
    let terminal = pm_registry::finish_pm_delivery_receipt(
        receipt_path,
        operation_id,
        &first.target_session_id,
        body_sha256,
        pm_registry::PmDeliveryReceiptStatus::Ambiguous,
        Some("replayed_open_prepared"),
    )
    .map_err(|error| format!("pm pane send: open operation reconciliation failed: {error}"))?;
    Ok(match terminal {
        pm_registry::PmDeliveryReceiptStatus::Verified => PmDeliveryReplayOutcome::Verified,
        _ => PmDeliveryReplayOutcome::Refused(
            "pm pane send: operation may already have staged its prompt body".to_string(),
        ),
    })
}

fn pm_delivery_replay_event(
    outcome: PmDeliveryReplayOutcome,
    missing_error: String,
) -> BackendEvent {
    match outcome {
        PmDeliveryReplayOutcome::Verified => BackendEvent::PaneSendResult {
            ok: true,
            window_id: None,
            error: None,
        },
        PmDeliveryReplayOutcome::Missing => BackendEvent::PaneSendResult {
            ok: false,
            window_id: None,
            error: Some(missing_error),
        },
        PmDeliveryReplayOutcome::Refused(error) => BackendEvent::PaneSendResult {
            ok: false,
            window_id: None,
            error: Some(error),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn defer_pm_delivery_replay_or_refusal(
    blocking_tasks: &BlockingTaskSpawner,
    receipt_path: PathBuf,
    operation_id: String,
    principal_session_id: String,
    target_window_id: String,
    body_sha256: String,
    reservations: PmSubmitReservationRegistry,
    responder: AgentPmSendResponder,
    missing_error: String,
) -> Result<(), String> {
    blocking_tasks.try_spawn(move || {
        let _operation_deadline =
            gwt_core::operation_deadline::ScopedOperationDeadline::enter(responder.deadline());
        let outcome = reconcile_pm_delivery_replay(
            &receipt_path,
            &operation_id,
            &principal_session_id,
            &target_window_id,
            &body_sha256,
        );
        let event = match outcome {
            Ok(PmDeliveryReplayOutcome::Verified) => {
                release_pm_submit(&reservations, &target_window_id, &operation_id);
                pm_delivery_replay_event(PmDeliveryReplayOutcome::Verified, missing_error)
            }
            Ok(outcome) => pm_delivery_replay_event(outcome, missing_error),
            Err(error) => BackendEvent::PaneSendResult {
                ok: false,
                window_id: None,
                error: Some(error),
            },
        };
        let _ = responder.send(event);
    })
}

/// Revalidate the full durable PM registration and exact PTY generation at
/// the mutation boundary. Each successful snapshot linearizes one physical
/// write; locks are released before OS I/O so GUI registration/teardown never
/// waits behind a stalled PTY. Later rotations reject every later write.
#[allow(clippy::too_many_arguments)]
fn write_registered_pm_pty(
    writers: &PtyWriterRegistry,
    window_id: &str,
    expected_pty: &Arc<gwt_terminal::PtyHandle>,
    prefs_path: &Path,
    registration: &PmRegistration,
    bytes: &[u8],
    cancellation: Option<&AgentPmSendResponder>,
    write: impl FnOnce(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    if cancellation.is_some_and(|cancellation| !cancellation.mutation_is_current()) {
        return Err("PM delivery deadline expired before input mutation".to_string());
    }
    let exact_registration =
        pm_registry::exact_pm_registration_is_current(prefs_path, registration)
            .map_err(|error| format!("PM registration revalidation failed: {error}"))?;
    if !exact_registration {
        return Err("registered PM authority changed before input mutation".to_string());
    }
    {
        let current = writers
            .read()
            .map_err(|_| "PM delivery PTY registry is unavailable".to_string())?;
        let Some(writer) = current.get(window_id) else {
            return Err(format!("no live runtime for pane {window_id}"));
        };
        if !Arc::ptr_eq(writer, expected_pty) {
            return Err(format!("pane {window_id} runtime generation changed"));
        }
    }
    if cancellation.is_some_and(|cancellation| !cancellation.mutation_is_current()) {
        return Err(format!(
            "PM delivery deadline expired before pane {window_id} input mutation"
        ));
    }
    write(bytes)
}

fn pm_delivery_receipt_is_verified(
    receipt_path: &Path,
    operation_id: &str,
    target_session_id: &str,
    body_sha256: &str,
) -> Result<bool, String> {
    pm_registry::load_pm_delivery_receipts(receipt_path)
        .map_err(|error| format!("PM delivery acknowledgement is unavailable: {error}"))
        .map(|receipts| {
            receipts.iter().rev().any(|receipt| {
                receipt.operation_id == operation_id
                    && receipt.target_session_id == target_session_id
                    && receipt.body_sha256 == body_sha256
                    && receipt.status == pm_registry::PmDeliveryReceiptStatus::Verified
            })
        })
}

fn pm_wake_submit_is_acknowledged(
    loop_path: &Path,
    receipt_path: &Path,
    operation_id: &str,
    target_session_id: &str,
    body_sha256: &str,
) -> Result<bool, String> {
    let loop_acknowledged = pm_registry::load_pm_loop_state(loop_path)
        .map_err(|error| format!("PM wake acknowledgement is unavailable: {error}"))
        .map(|state| {
            state.last_delivery_operation_id.as_deref() == Some(operation_id)
                && state.last_delivery_submitted_at.is_some()
        })?;
    Ok(loop_acknowledged
        || pm_delivery_receipt_is_verified(
            receipt_path,
            operation_id,
            target_session_id,
            body_sha256,
        )?)
}

fn pm_wake_submit_was_rejected(loop_path: &Path, operation_id: &str) -> Result<bool, String> {
    pm_registry::load_pm_loop_state(loop_path)
        .map_err(|error| format!("PM wake acknowledgement is unavailable: {error}"))
        .map(|state| state.last_delivery_rejected_operation_id.as_deref() == Some(operation_id))
}

fn pm_pane_submit_is_acknowledged(receipt_path: &Path, operation_id: &str) -> Result<bool, String> {
    pm_registry::load_pm_delivery_receipts(receipt_path)
        .map_err(|error| format!("PM delivery acknowledgement is unavailable: {error}"))
        .map(|receipts| {
            receipts.iter().rev().any(|receipt| {
                receipt.operation_id == operation_id
                    && receipt.status == pm_registry::PmDeliveryReceiptStatus::Verified
            })
        })
}

fn finish_pm_wake_receipt(
    receipt_path: &Path,
    decision: &PmWakeDecision,
    status: pm_registry::PmDeliveryReceiptStatus,
    error_code: &'static str,
) -> Result<pm_registry::PmDeliveryReceiptStatus, String> {
    pm_registry::finish_pm_delivery_receipt(
        receipt_path,
        &decision.operation_id,
        &decision.registration.session_id,
        &decision.body_sha256,
        status,
        Some(error_code),
    )
    .map_err(|error| format!("PM wake terminal receipt commit failed: {error}"))
}

fn reconcile_verified_pm_wake_receipt(
    loop_path: &Path,
    receipt_path: &Path,
    decision: &PmWakeDecision,
    status: Result<pm_registry::PmDeliveryReceiptStatus, String>,
) -> Option<PmWakeDeliveryResult> {
    match status {
        Ok(pm_registry::PmDeliveryReceiptStatus::Verified) => Some(
            match commit_verified_pm_wake(
                loop_path,
                receipt_path,
                &decision.operation_id,
                &decision.registration.session_id,
                &decision.body_sha256,
            ) {
                Ok(()) => PmWakeDeliveryResult::Verified,
                Err(error) => PmWakeDeliveryResult::CommitRetryable { error },
            },
        ),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(%error, operation_id = %decision.operation_id, "PM wake receipt could not be terminalized");
            None
        }
    }
}

fn commit_verified_pm_wake(
    loop_path: &Path,
    receipt_path: &Path,
    operation_id: &str,
    target_session_id: &str,
    body_sha256: &str,
) -> Result<(), String> {
    let receipt_acknowledged = !body_sha256.is_empty()
        && pm_delivery_receipt_is_verified(
            receipt_path,
            operation_id,
            target_session_id,
            body_sha256,
        )
        .unwrap_or(false);
    let (_, matched) = pm_registry::mutate_pm_loop_state(loop_path, |state| {
        let loop_acknowledged = state.last_delivery_operation_id.as_deref() == Some(operation_id)
            && state.last_delivery_submitted_at.is_some();
        if !loop_acknowledged && !receipt_acknowledged {
            return false;
        }
        let submitted_at = state.last_delivery_submitted_at.clone().unwrap_or_else(|| {
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        });
        if receipt_acknowledged && !loop_acknowledged {
            state.last_delivery_operation_id = Some(operation_id.to_string());
            state.last_delivery_submitted_at = Some(submitted_at.clone());
        }
        state.consecutive_continuations = 0;
        state.pending_own_block = false;
        state.last_wake_at = Some(submitted_at);
        if state.pending_delivery_operation_id.as_deref() == Some(operation_id) {
            state.pending_delivery_operation_id = None;
            state.pending_delivery_window_id = None;
            state.pending_delivery_session_id = None;
            state.pending_delivery_body_sha256 = None;
        }
        true
    })
    .map_err(|error| format!("PM wake clock commit failed: {error}"))?;
    if matched {
        Ok(())
    } else {
        Err("exact PM wake submit acknowledgement is no longer available".to_string())
    }
}

fn reconcile_crash_surviving_pm_wake(
    project_root: &Path,
    loop_path: &Path,
    state: &pm_registry::PmLoopState,
) -> bool {
    let Some(operation_id) = state.pending_delivery_operation_id.as_deref() else {
        return true;
    };
    let Some(target_session_id) = state.pending_delivery_session_id.as_deref() else {
        return false;
    };
    let loop_acknowledged = state.last_delivery_operation_id.as_deref() == Some(operation_id)
        && state.last_delivery_submitted_at.is_some();
    let body_sha256 = match state.pending_delivery_body_sha256.as_deref() {
        Some(body_sha256) => body_sha256,
        None if loop_acknowledged => "",
        None => return false,
    };
    let receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(project_root);
    if let Err(error) = commit_verified_pm_wake(
        loop_path,
        &receipt_path,
        operation_id,
        target_session_id,
        body_sha256,
    ) {
        tracing::warn!(%error, operation_id, "PM wake restart reconciliation could not commit the exact acknowledgement");
        return false;
    }
    true
}

fn run_pm_wake_delivery(
    writers: &PtyWriterRegistry,
    expected_pty: Option<&Arc<gwt_terminal::PtyHandle>>,
    decision: &PmWakeDecision,
    submit_only: bool,
    commit_only: bool,
) -> PmWakeDeliveryResult {
    let loop_path = pm_registry::pm_loop_state_path_for_repo_path(&decision.project_root);
    let receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(&decision.project_root);
    if commit_only {
        return match commit_verified_pm_wake(
            &loop_path,
            &receipt_path,
            &decision.operation_id,
            &decision.registration.session_id,
            &decision.body_sha256,
        ) {
            Ok(()) => PmWakeDeliveryResult::Verified,
            Err(error) => PmWakeDeliveryResult::CommitRetryable { error },
        };
    }
    match pm_wake_submit_is_acknowledged(
        &loop_path,
        &receipt_path,
        &decision.operation_id,
        &decision.registration.session_id,
        &decision.body_sha256,
    ) {
        Ok(true) => {
            return match commit_verified_pm_wake(
                &loop_path,
                &receipt_path,
                &decision.operation_id,
                &decision.registration.session_id,
                &decision.body_sha256,
            ) {
                Ok(()) => PmWakeDeliveryResult::Verified,
                Err(error) => PmWakeDeliveryResult::CommitRetryable { error },
            }
        }
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, "PM wake acknowledgement could not be read; retaining quarantine");
            return PmWakeDeliveryResult::SubmitUnverified;
        }
    }
    match pm_wake_submit_was_rejected(&loop_path, &decision.operation_id) {
        Ok(true) => return PmWakeDeliveryResult::SubmitUnverified,
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, "PM wake rejection evidence could not be read; retaining quarantine");
            return PmWakeDeliveryResult::SubmitUnverified;
        }
    }
    let Some(expected_pty) = expected_pty else {
        return PmWakeDeliveryResult::Terminal {
            error: "PM wake has no exact PTY generation".to_string(),
        };
    };
    let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&decision.project_root);
    let prepared_receipt = pm_registry::PmDeliveryReceipt {
        operation_id: decision.operation_id.clone(),
        recorded_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        status: pm_registry::PmDeliveryReceiptStatus::Prepared,
        principal_session_id: decision.registration.session_id.clone(),
        target_window_id: decision.window_id.clone(),
        target_session_id: decision.registration.session_id.clone(),
        body_sha256: decision.body_sha256.clone(),
        error_code: None,
    };
    match pm_registry::prepare_pm_delivery_receipt(&receipt_path, &prepared_receipt) {
        Ok(pm_registry::PmDeliveryPrepareOutcome::Prepared) => {}
        Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
            pm_registry::PmDeliveryReceiptStatus::Prepared,
        )) => {
            let status = finish_pm_wake_receipt(
                &receipt_path,
                decision,
                pm_registry::PmDeliveryReceiptStatus::Ambiguous,
                "replayed_open_prepared",
            );
            return reconcile_verified_pm_wake_receipt(&loop_path, &receipt_path, decision, status)
                .unwrap_or(PmWakeDeliveryResult::SubmitUnverified);
        }
        Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
            pm_registry::PmDeliveryReceiptStatus::Verified,
        )) => {
            return match commit_verified_pm_wake(
                &loop_path,
                &receipt_path,
                &decision.operation_id,
                &decision.registration.session_id,
                &decision.body_sha256,
            ) {
                Ok(()) => PmWakeDeliveryResult::Verified,
                Err(error) => PmWakeDeliveryResult::CommitRetryable { error },
            };
        }
        Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(status)) => {
            return PmWakeDeliveryResult::Terminal {
                error: format!("PM wake delivery receipt is already terminal: {status:?}"),
            };
        }
        Err(error) => {
            return PmWakeDeliveryResult::Terminal {
                error: format!("PM wake Prepared receipt commit failed: {error}"),
            };
        }
    }
    let text = if submit_only {
        "\r"
    } else {
        decision.prompt.as_str()
    };
    let attempts = if submit_only {
        1
    } else {
        PM_WAKE_SUBMIT_ATTEMPTS
    };
    let transaction = match Arc::clone(expected_pty).reserve_input_transaction() {
        Ok(transaction) => transaction,
        Err(error) => {
            let status = finish_pm_wake_receipt(
                &receipt_path,
                decision,
                pm_registry::PmDeliveryReceiptStatus::Refused,
                "input_transaction_unavailable",
            );
            if let Some(result) =
                reconcile_verified_pm_wake_receipt(&loop_path, &receipt_path, decision, status)
            {
                return result;
            }
            return PmWakeDeliveryResult::Terminal {
                error: format!("PM wake input transaction unavailable: {error}"),
            };
        }
    };
    let successful_writes = std::cell::Cell::new(0_usize);
    let body_write_attempted = std::cell::Cell::new(false);
    let outcome = super::pty_io::drive_verified_pane_submit(
        text,
        attempts,
        |bytes| {
            if bytes == b"\r" && pm_wake_submit_was_rejected(&loop_path, &decision.operation_id)? {
                return Err(
                    "PM wake prompt did not match its protected composer reservation".to_string(),
                );
            }
            write_registered_pm_pty(
                writers,
                &decision.window_id,
                expected_pty,
                &prefs_path,
                &decision.registration,
                bytes,
                None,
                |bytes| {
                    if bytes != b"\r" {
                        body_write_attempted.set(true);
                    }
                    transaction
                        .write_input(bytes)
                        .map_err(|error| error.to_string())
                },
            )?;
            successful_writes.set(successful_writes.get().saturating_add(1));
            Ok(())
        },
        thread::sleep,
        || {
            pm_wake_submit_is_acknowledged(
                &loop_path,
                &receipt_path,
                &decision.operation_id,
                &decision.registration.session_id,
                &decision.body_sha256,
            )
        },
    );
    match outcome {
        Ok(super::pty_io::VerifiedPaneSubmitOutcome::Verified { .. }) => {
            match commit_verified_pm_wake(
                &loop_path,
                &receipt_path,
                &decision.operation_id,
                &decision.registration.session_id,
                &decision.body_sha256,
            ) {
                Ok(()) => PmWakeDeliveryResult::Verified,
                Err(error) => PmWakeDeliveryResult::CommitRetryable { error },
            }
        }
        Ok(super::pty_io::VerifiedPaneSubmitOutcome::Unverified { .. }) => {
            let status = finish_pm_wake_receipt(
                &receipt_path,
                decision,
                pm_registry::PmDeliveryReceiptStatus::Ambiguous,
                "submit_unverified",
            );
            reconcile_verified_pm_wake_receipt(&loop_path, &receipt_path, decision, status)
                .unwrap_or(PmWakeDeliveryResult::SubmitUnverified)
        }
        Err(error)
            if pm_wake_submit_was_rejected(&loop_path, &decision.operation_id).unwrap_or(true) =>
        {
            tracing::warn!(%error, "PM wake composer verification failed; retaining pane quarantine");
            let status = finish_pm_wake_receipt(
                &receipt_path,
                decision,
                pm_registry::PmDeliveryReceiptStatus::Ambiguous,
                "composer_hash_mismatch",
            );
            reconcile_verified_pm_wake_receipt(&loop_path, &receipt_path, decision, status)
                .unwrap_or(PmWakeDeliveryResult::SubmitUnverified)
        }
        Err(error) if successful_writes.get() > 0 || body_write_attempted.get() || submit_only => {
            tracing::warn!(%error, "PM wake submit failed after staging the prompt body; retaining CR-only recovery");
            let status = finish_pm_wake_receipt(
                &receipt_path,
                decision,
                pm_registry::PmDeliveryReceiptStatus::Ambiguous,
                "body_staged_or_unknown",
            );
            reconcile_verified_pm_wake_receipt(&loop_path, &receipt_path, decision, status)
                .unwrap_or(PmWakeDeliveryResult::SubmitUnverified)
        }
        Err(error) => {
            let status = finish_pm_wake_receipt(
                &receipt_path,
                decision,
                pm_registry::PmDeliveryReceiptStatus::Refused,
                "mutation_refused",
            );
            reconcile_verified_pm_wake_receipt(&loop_path, &receipt_path, decision, status)
                .unwrap_or(PmWakeDeliveryResult::Terminal { error })
        }
    }
}

/// What one monitor inbox snapshot contributes to the wake decision: every
/// issue the monitor is holding, plus an extra marker for the rows a human
/// must look at. A signal string appearing for the first time is "new
/// activity"; the sets are compared, never interpreted.
fn pm_wake_signals(inbox: &[gwt::IssueMonitorInboxItem]) -> std::collections::BTreeSet<String> {
    let mut signals = std::collections::BTreeSet::new();
    for item in inbox {
        signals.insert(format!("issue:{}", item.issue.number));
        if item.state == gwt::MonitorInboxState::NeedsHuman {
            signals.insert(format!("needs_human:{}", item.issue.number));
        }
    }
    signals
}

/// An actively-looping PM picks new events up in its own next cycle, and a PM
/// a human just prompted is busy with that conversation — the wake is only
/// for a loop quiet on both clocks (parked on the budget cap, or dead after a
/// within-floor stop, with no recent user contact). Same interval knob as the
/// Stop-gate driver.
fn pm_wake_loop_is_quiet(state: &pm_registry::PmLoopState, interval_secs: u64, now: &str) -> bool {
    let instant_is_quiet = |instant: Option<&str>| {
        let Some(instant) = instant else {
            return true;
        };
        match (
            chrono::DateTime::parse_from_rfc3339(now),
            chrono::DateTime::parse_from_rfc3339(instant),
        ) {
            (Ok(now_t), Ok(instant_t)) => {
                (now_t - instant_t).num_seconds()
                    >= i64::try_from(interval_secs).unwrap_or(i64::MAX)
            }
            _ => true,
        }
    };
    instant_is_quiet(state.last_continued_at.as_deref())
        && instant_is_quiet(state.last_user_prompt_at.as_deref())
        && instant_is_quiet(state.last_wake_at.as_deref())
}

/// Who asked for the PM.
///
/// SPEC-3431 FR-002's `auto_start` opt-out scopes to "opening a project starts
/// the PM automatically". It is not a lock on the user: FR-021 requires the PM
/// launcher to start a stopped PM when clicked, and FR-003 promises a crash
/// resumes. Collapsing both into one gate leaves the launcher and the Restart
/// button silently dead with no way back short of editing pm.json by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmEnsureTrigger {
    /// Project open / startup restore. Honours the opt-out.
    Automatic,
    /// Launcher click, Restart, crash recovery. Ignores the opt-out.
    Explicit,
}

#[cfg(test)]
type PmRegistrationCommitHook = Box<dyn FnOnce() -> Result<(), String> + 'static>;

#[cfg(test)]
thread_local! {
    static PM_REGISTRATION_COMMIT_HOOK:
        std::cell::RefCell<Option<PmRegistrationCommitHook>> = const {
            std::cell::RefCell::new(None)
        };
}

#[cfg(test)]
pub(crate) fn set_pm_registration_commit_hook_for_test(
    hook: impl FnOnce() -> Result<(), String> + 'static,
) {
    PM_REGISTRATION_COMMIT_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn invoke_pm_registration_commit_hook() -> Result<(), String> {
    PM_REGISTRATION_COMMIT_HOOK.with(|slot| slot.borrow_mut().take().map_or(Ok(()), |hook| hook()))
}

#[cfg(not(test))]
fn invoke_pm_registration_commit_hook() -> Result<(), String> {
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedPmRegistration {
    project_root: PathBuf,
    work_event_root: PathBuf,
    prefs_path: PathBuf,
    candidate: PmRegistration,
    commit_mode: PmRegistrationCommitMode,
    operation_id: String,
}

#[derive(Debug, Clone)]
enum PmRegistrationCommitMode {
    ReplaceStaleOrEmpty,
    ReplaceExact(PmRegistration),
}

impl PreparedPmRegistration {
    pub(crate) fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub(crate) fn work_event_root(&self) -> &Path {
        &self.work_event_root
    }

    pub(crate) fn operation_id(&self) -> &str {
        &self.operation_id
    }
}

impl AppRuntime {
    /// SPEC-3431 FR-001/FR-002: ensure the resident PM pane for `tab_id`.
    ///
    /// Focuses a live PM instead of spawning a duplicate, resumes a stale
    /// registration's conversation when the durable session is still
    /// materializable, and otherwise performs a fresh silent spawn. Never
    /// spawns implementation agents.
    pub(crate) fn ensure_pm_agent_for_tab(
        &mut self,
        tab_id: &str,
        trigger: PmEnsureTrigger,
    ) -> Vec<OutboundEvent> {
        self.ensure_pm_agent_for_tab_with_bounds(tab_id, None, trigger)
    }

    /// [`Self::ensure_pm_agent_for_tab`] with the caller's visible canvas
    /// bounds, so an existing PM is framed in the viewport (FR-019).
    pub(crate) fn ensure_pm_agent_for_tab_with_bounds(
        &mut self,
        tab_id: &str,
        canvas_bounds: Option<WindowGeometry>,
        trigger: PmEnsureTrigger,
    ) -> Vec<OutboundEvent> {
        #[cfg(test)]
        if !test_gate::PM_ENSURE_ENABLED.with(|cell| cell.get()) {
            return Vec::new();
        }
        let mut events = self.ensure_pm_agent_events(tab_id, canvas_bounds, trigger);
        // FR-026: the ensure gate is where the PM's live state changes, so it
        // is also where the settings panel learns about it. Skipped ensures
        // (opt-out, non-Git tab, backoff floor) still report — "not running"
        // is exactly the state the panel has to show.
        if self.active_tab_id.as_deref() == Some(tab_id) {
            events.extend(self.pm_status_broadcast_events());
        }
        events
    }

    fn ensure_pm_agent_events(
        &mut self,
        tab_id: &str,
        canvas_bounds: Option<WindowGeometry>,
        trigger: PmEnsureTrigger,
    ) -> Vec<OutboundEvent> {
        let Some(tab) = self.tab(tab_id) else {
            tracing::info!(tab_id, "PM ensure skipped: no such tab");
            return Vec::new();
        };
        if tab.kind != gwt::ProjectKind::Git || tab.migration_pending {
            tracing::info!(
                tab_id,
                migration_pending = tab.migration_pending,
                "PM ensure skipped: tab is not a migration-clear Git project"
            );
            return Vec::new();
        }
        let project_root = tab.project_root.clone();
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let prefs = match pm_registry::load_pm_prefs(&prefs_path) {
            Ok(prefs) => prefs,
            Err(error) => {
                tracing::warn!(
                    path = %prefs_path.display(),
                    %error,
                    "failed to load PM prefs; skipping PM ensure"
                );
                return Vec::new();
            }
        };
        self.sync_pm_session_cache(&project_root, prefs.registration.as_ref());
        if trigger == PmEnsureTrigger::Automatic && !prefs.settings.auto_start {
            tracing::info!(
                project_root = %project_root.display(),
                "PM ensure skipped: auto_start is opted out for this project"
            );
            return Vec::new();
        }
        let Some(registration) = prefs.registration else {
            tracing::info!(
                project_root = %project_root.display(),
                "PM ensure: no registration yet, spawning the resident PM"
            );
            return self.spawn_pm_agent(tab_id, &project_root);
        };
        if let Some(window_id) = self.live_pm_window_id(&registration.session_id) {
            // FR-019: the PM launcher must always land the user on the PM, so
            // focusing frames it in the viewport rather than only raising it.
            return self.focus_existing_live_work_agent_events(&window_id, canvas_bounds);
        }
        // FR-003 crash-loop damper: while the backoff floor is in the future
        // the automatic ladder must not respawn. Scoped to `Automatic` for the
        // same reason as the auto_start opt-out above — FR-021 requires the
        // explicit launcher/Restart click to start a stopped PM regardless.
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        if trigger == PmEnsureTrigger::Automatic
            && !pm_registry::pm_respawn_allowed(&registration, &now)
        {
            return Vec::new();
        }
        // Stale registration (FR-003): resume the same conversation when the
        // durable session is still materializable; otherwise fall back to a
        // fresh spawn whose completion replaces the stale registration.
        let session_path = self
            .sessions_dir
            .join(format!("{}.toml", registration.session_id));
        if let Ok(session) = gwt_agent::Session::load_and_migrate(&session_path) {
            if session.worktree_path.exists() {
                let before = self.pm_window_ids(tab_id);
                let events =
                    self.spawn_registered_agent_session(tab_id, session, None, PM_WINDOW_GEOMETRY);
                self.mark_new_pm_windows(tab_id, &before, &project_root);
                return events;
            }
        }
        self.spawn_pm_agent(tab_id, &project_root)
    }

    /// SPEC-3431 FR-026: the PM settings snapshot for the active project tab.
    ///
    /// `None` when there is no Git project to configure — the panel then keeps
    /// showing whatever it last had rather than being fed an empty project's
    /// defaults as if they were this one's.
    pub(crate) fn pm_status_event(&self) -> Option<BackendEvent> {
        let project_root = self.active_pm_project_root()?;
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let prefs = pm_registry::load_pm_prefs(&prefs_path).unwrap_or_default();
        let configured = prefs.settings.launch_profile_or_default();
        // Liveness is the pane registry's answer, not the file's: a
        // registration whose pane is gone is a stale record, and reporting it
        // as running would make the panel offer a restart for nothing.
        let running = prefs
            .registration
            .as_ref()
            .filter(|registration| self.pm_registration_is_live(registration));
        Some(BackendEvent::PmStatus {
            auto_start: prefs.settings.auto_start,
            agent_options: Self::pm_agent_options(&configured.agent_id),
            configured_agent_id: configured.agent_id,
            configured_model: configured.model,
            running_agent_id: running.map(|registration| registration.agent_id.clone()),
            is_running: running.is_some(),
        })
    }

    /// The PM settings snapshot as a broadcast, for the call sites that change
    /// PM state. Every PM state transition must pass through here — the panel
    /// has no other source of truth, so a silent transition leaves it stale.
    pub(crate) fn pm_status_broadcast_events(&self) -> Vec<OutboundEvent> {
        self.pm_status_event()
            .map(OutboundEvent::broadcast)
            .into_iter()
            .collect()
    }

    /// Selectable PM agents: the ones that can resolve `$gwt-pm`, narrowed to
    /// what is actually installed.
    ///
    /// `configured` is always offered even when it is not on PATH, so the
    /// picker can still show (and keep) the project's current choice instead of
    /// silently presenting a different agent as the configured one.
    fn pm_agent_options(configured: &str) -> Vec<PmAgentOption> {
        pm_registry::PM_SUPPORTED_AGENTS
            .iter()
            .filter(|id| **id == configured || which::which(id).is_ok())
            .map(|id| PmAgentOption {
                id: (*id).to_string(),
                name: gwt_agent::builtin_agent_descriptor_for_command(id)
                    .map_or_else(|| (*id).to_string(), |d| d.id.display_name().to_string()),
            })
            .collect()
    }

    fn active_pm_project_root(&self) -> Option<PathBuf> {
        let tab_id = self.active_tab_id.clone()?;
        self.tab(&tab_id).map(|tab| tab.project_root.clone())
    }

    /// SPEC-3431 FR-026/FR-002: persist the auto-start opt-out.
    ///
    /// Deliberately does not touch the running pane. The flag decides whether
    /// opening the project starts a PM; treating it as a stop switch would end
    /// a conversation the user only meant to stop auto-starting next time.
    pub(crate) fn set_pm_auto_start_events(&mut self, enabled: bool) -> Vec<OutboundEvent> {
        let Some(project_root) = self.active_pm_project_root() else {
            return Vec::new();
        };
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        if let Err(error) = pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.settings.auto_start = enabled;
        }) {
            tracing::warn!(%error, "failed to persist the PM auto-start setting");
            return Vec::new();
        }
        self.pm_status_broadcast_events()
    }

    /// SPEC-3431 FR-026: persist what the next PM start runs as.
    ///
    /// An agent without a managed `gwt-pm` skills mirror is refused rather than
    /// stored: `PmSettings::launch_profile_or_default` would silently fall back
    /// at launch time, leaving the panel claiming a configuration the PM never
    /// actually uses. The running pane is untouched — applying the change is
    /// [`Self::restart_pm_agent_events`].
    pub(crate) fn set_pm_launch_profile_events(
        &mut self,
        agent_id: &str,
        model: Option<String>,
        reasoning: Option<String>,
    ) -> Vec<OutboundEvent> {
        let Some(project_root) = self.active_pm_project_root() else {
            return Vec::new();
        };
        if !pm_registry::pm_agent_is_supported(agent_id) {
            tracing::warn!(
                agent_id,
                "rejected PM launch profile: the agent cannot resolve the gwt-pm skill"
            );
            return Vec::new();
        }
        let empty_to_none = |value: Option<String>| value.filter(|value| !value.trim().is_empty());
        let profile = PmLaunchProfile {
            agent_id: agent_id.to_string(),
            model: empty_to_none(model),
            reasoning: empty_to_none(reasoning),
            version: None,
        };
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        if let Err(error) = pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            prefs.settings.launch_profile = Some(profile.clone());
        }) {
            tracing::warn!(%error, "failed to persist the PM launch profile");
            return Vec::new();
        }
        self.pm_status_broadcast_events()
    }

    /// SPEC-3431 FR-026: apply the configured profile by restarting the PM.
    ///
    /// Deregisters first, on purpose: the close path treats a registered PM's
    /// close as an intentional stop and reaps a clean PM worktree with it
    /// (T-016). A restart is not a stop — the worktree holds the PM's own
    /// notes — so clearing the registration up front makes that reap a no-op
    /// and leaves the worktree for the successor.
    pub(crate) fn restart_pm_agent_events(&mut self) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.clone() else {
            return Vec::new();
        };
        let Some(project_root) = self.tab(&tab_id).map(|tab| tab.project_root.clone()) else {
            return Vec::new();
        };
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let registration = pm_registry::load_pm_prefs(&prefs_path)
            .ok()
            .and_then(|prefs| prefs.registration);
        let mut events = Vec::new();
        if let Some(registration) = registration {
            match pm_registry::deregister_pm(&prefs_path, &registration.session_id) {
                Ok(_) => self.sync_pm_session_cache(&project_root, None),
                Err(error) => {
                    tracing::warn!(%error, "failed to clear the PM registration for a restart");
                    return Vec::new();
                }
            }
            if let Some(window_id) = self.live_pm_window_id(&registration.session_id) {
                events.extend(self.close_window_events(&window_id));
            }
        }
        // The ensure gate broadcasts the post-restart pm_status itself, so the
        // panel is refreshed exactly once rather than twice per restart.
        events.extend(self.ensure_pm_agent_for_tab(&tab_id, PmEnsureTrigger::Explicit));
        events
    }

    /// SPEC-3431 T-093 (FR-012): decide whether this inbox snapshot must wake
    /// the resident PM, using `now` as the quiet-loop clock.
    ///
    /// The first snapshot per project is a baseline — GUI startup must not
    /// replay a long-lived backlog as if it just happened. After that, a
    /// signal never seen before wakes the PM iff the Monitor is enabled, a
    /// registered PM pane is live, and the resident loop has gone quiet.
    /// A delta suppressed only by an active loop is retained (not consumed),
    /// so a loop that dies inside its floor is still revived by the next
    /// snapshot; every other outcome consumes the delta.
    pub(crate) fn pm_wake_decision_at(
        &mut self,
        project_root: &Path,
        inbox: &[gwt::IssueMonitorInboxItem],
        now: &str,
    ) -> Option<PmWakeDecision> {
        let signals = pm_wake_signals(inbox);
        if self.pending_pm_wake_deliveries.contains_key(project_root) {
            return None;
        }
        let Some(seen) = self.pm_wake_seen.get(project_root) else {
            self.pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        };
        let fresh: Vec<String> = signals.difference(seen).cloned().collect();
        if fresh.is_empty() {
            self.pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        }
        // Monitor off = the user parked the project (the Stop-gate driver's
        // rule). Consume the delta: re-enabling is a GUI action with a human
        // present, not a moment to replay stale news.
        let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let monitor_enabled = gwt::load_issue_monitor_prefs(&monitor_prefs_path)
            .map(|prefs| prefs.enabled)
            .unwrap_or(false);
        if !monitor_enabled {
            self.pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        }
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let prefs = match pm_registry::load_pm_prefs(&prefs_path) {
            Ok(prefs) => prefs,
            Err(error) => {
                tracing::warn!(%error, "PM wake skipped: pm prefs unreadable");
                return None;
            }
        };
        let Some(registration) = prefs.registration else {
            // No PM to wake; a later PM start reads status in its bootstrap.
            self.pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        };
        let Some(window_id) = self.live_pm_window_id(&registration.session_id) else {
            // A dead PM is the crash-resume path's job, never the wake's.
            self.pm_wake_seen
                .insert(project_root.to_path_buf(), signals);
            return None;
        };
        let interval_secs = prefs.settings.loop_interval_secs_clamped();
        let loop_path = pm_registry::pm_loop_state_path_for_repo_path(project_root);
        let mut loop_state = pm_registry::load_pm_loop_state(&loop_path).unwrap_or_default();
        if loop_state.pending_delivery_operation_id.is_some()
            && !reconcile_crash_surviving_pm_wake(project_root, &loop_path, &loop_state)
        {
            tracing::warn!(
                window_id = ?loop_state.pending_delivery_window_id,
                "PM wake skipped: a crash-surviving delivery reservation needs pane recovery"
            );
            return None;
        }
        loop_state = pm_registry::load_pm_loop_state(&loop_path).unwrap_or(loop_state);
        if !pm_wake_loop_is_quiet(&loop_state, interval_secs, now) {
            // Actively looping: its own next cycle reconciles this. Keep the
            // delta so a floor-stopped loop is revived by the next snapshot.
            return None;
        }
        let mut reasons = fresh;
        reasons.truncate(5);
        let (operation_id, prompt, body_sha256) = protected_pm_wake_prompt(format!(
            "[gwt] Issue Monitor activity while the resident PM loop was idle ({}). \
             Run one reconcile cycle now: read a fresh `issue.monitor.status` snapshot, \
             triage the new items, and report the milestone digest.",
            reasons.join(", "),
        ));
        let decision = PmWakeDecision {
            window_id,
            prompt,
            project_root: project_root.to_path_buf(),
            registration,
            operation_id,
            body_sha256,
            kind: PmWakeKind::Delta { signals },
        };
        if !reserve_pm_submit(
            &self.pm_submit_reservations,
            &decision.window_id,
            &decision.operation_id,
        )
        .ok()?
        {
            return None;
        }
        match pm_registry::prepare_pm_wake_delivery(
            &loop_path,
            &decision.operation_id,
            &decision.window_id,
            &decision.registration.session_id,
            &decision.body_sha256,
        ) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                release_pm_submit(
                    &self.pm_submit_reservations,
                    &decision.window_id,
                    &decision.operation_id,
                );
                return None;
            }
        }
        self.pending_pm_wake_deliveries.insert(
            project_root.to_path_buf(),
            PendingPmWakeDelivery {
                decision: decision.clone(),
                phase: PmWakeDeliveryPhase::InFlight,
                expected_pty: None,
            },
        );
        Some(decision)
    }

    /// FR-108(b) (T-201, Issue #3505): the periodic wake — re-arm a quiet
    /// resident PM on the scheduled tick even when no new monitor signal
    /// arrived, as long as there is standing supervision work (running
    /// launches or a non-empty queue). The delta wake (T-093) covers "new
    /// things happened"; this covers "old things still need watching".
    ///
    /// The same quiet gate as the delta wake keeps the two from double-firing:
    /// an actively-looping or freshly-prompted PM is never interrupted, and a
    /// wake re-arms the loop so the next tick inside the interval is quiet-
    /// gated out.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn pm_periodic_wake_decision_at(
        &mut self,
        project_root: &Path,
        now: &str,
    ) -> Option<PmWakeDecision> {
        if self.pending_pm_wake_deliveries.contains_key(project_root) {
            return None;
        }
        let monitor_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let monitor_prefs = gwt::load_issue_monitor_prefs(&monitor_prefs_path).ok()?;
        let monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), monitor_prefs);
        self.pm_periodic_wake_decision_for_monitor_at(project_root, &monitor, now)
    }

    pub(crate) fn pm_periodic_wake_decision_for_monitor_at(
        &mut self,
        project_root: &Path,
        monitor: &gwt::IssueMonitorState,
        now: &str,
    ) -> Option<PmWakeDecision> {
        if !monitor.config.enabled {
            return None;
        }
        let status = monitor.agent_status();
        if status.active_launches.is_empty()
            && status.queue.is_empty()
            && status.needs_human.is_empty()
        {
            return None;
        }
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let prefs = pm_registry::load_pm_prefs(&prefs_path).ok()?;
        let registration = prefs.registration?;
        let window_id = self.live_pm_window_id(&registration.session_id)?;
        let interval_secs = prefs.settings.loop_interval_secs_clamped();
        let loop_path = pm_registry::pm_loop_state_path_for_repo_path(project_root);
        let mut loop_state = pm_registry::load_pm_loop_state(&loop_path).unwrap_or_default();
        if loop_state.pending_delivery_operation_id.is_some()
            && !reconcile_crash_surviving_pm_wake(project_root, &loop_path, &loop_state)
        {
            tracing::warn!(
                window_id = ?loop_state.pending_delivery_window_id,
                "periodic PM wake skipped: a crash-surviving delivery reservation needs pane recovery"
            );
            return None;
        }
        loop_state = pm_registry::load_pm_loop_state(&loop_path).unwrap_or(loop_state);
        if !pm_wake_loop_is_quiet(&loop_state, interval_secs, now) {
            return None;
        }
        let (operation_id, prompt, body_sha256) = protected_pm_wake_prompt(
            "[gwt] Scheduled supervision tick: run one PM reconcile cycle now — read a \
             fresh `issue.monitor.status` snapshot, check the running agents' \
             `last_activity_at` and any NeedsHuman rows, and report the milestone \
             digest."
                .to_string(),
        );
        let decision = PmWakeDecision {
            window_id,
            prompt,
            project_root: project_root.to_path_buf(),
            registration,
            operation_id,
            body_sha256,
            kind: PmWakeKind::Periodic,
        };
        if !reserve_pm_submit(
            &self.pm_submit_reservations,
            &decision.window_id,
            &decision.operation_id,
        )
        .ok()?
        {
            return None;
        }
        match pm_registry::prepare_pm_wake_delivery(
            &loop_path,
            &decision.operation_id,
            &decision.window_id,
            &decision.registration.session_id,
            &decision.body_sha256,
        ) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                release_pm_submit(
                    &self.pm_submit_reservations,
                    &decision.window_id,
                    &decision.operation_id,
                );
                return None;
            }
        }
        self.pending_pm_wake_deliveries.insert(
            project_root.to_path_buf(),
            PendingPmWakeDelivery {
                decision: decision.clone(),
                phase: PmWakeDeliveryPhase::InFlight,
                expected_pty: None,
            },
        );
        Some(decision)
    }

    pub(crate) fn pm_periodic_wake_events_for_monitor_at(
        &mut self,
        project_root: &Path,
        monitor: &gwt::IssueMonitorState,
        now: &str,
    ) -> Vec<OutboundEvent> {
        if self.resume_pending_pm_wake_delivery(project_root) {
            return Vec::new();
        }
        let Some(decision) =
            self.pm_periodic_wake_decision_for_monitor_at(project_root, monitor, now)
        else {
            return Vec::new();
        };
        if let Err(error) = self.start_pm_wake_delivery(decision.clone(), false, false) {
            return self.handle_pm_wake_delivery_complete(
                &decision.project_root,
                &decision.operation_id,
                PmWakeDeliveryResult::Terminal { error },
            );
        }
        Vec::new()
    }

    pub(crate) fn pm_periodic_wake_events_at(
        &mut self,
        project_root: &Path,
        now: &str,
    ) -> Vec<OutboundEvent> {
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let Ok(prefs) = gwt::load_issue_monitor_prefs(&prefs_path) else {
            return Vec::new();
        };
        let monitor = gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
        self.pm_periodic_wake_events_for_monitor_at(project_root, &monitor, now)
    }

    /// Execute the wake off the event loop. The signal fingerprint and wake
    /// clock remain untouched until the exact marker reaches UserPromptSubmit;
    /// an unverified submit reserves the body and retries only the CR.
    pub(crate) fn pm_wake_events(
        &mut self,
        project_root: &Path,
        inbox: &[gwt::IssueMonitorInboxItem],
    ) -> Vec<OutboundEvent> {
        if self.resume_pending_pm_wake_delivery(project_root) {
            return Vec::new();
        }
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let Some(decision) = self.pm_wake_decision_at(project_root, inbox, &now) else {
            return Vec::new();
        };
        if let Err(error) = self.start_pm_wake_delivery(decision.clone(), false, false) {
            return self.handle_pm_wake_delivery_complete(
                &decision.project_root,
                &decision.operation_id,
                PmWakeDeliveryResult::Terminal { error },
            );
        }
        Vec::new()
    }

    fn resume_pending_pm_wake_delivery(&mut self, project_root: &Path) -> bool {
        let Some(pending) = self.pending_pm_wake_deliveries.get(project_root).cloned() else {
            return false;
        };
        match pending.phase {
            PmWakeDeliveryPhase::InFlight => true,
            PmWakeDeliveryPhase::NeedsRecovery => {
                let loop_path = pm_registry::pm_loop_state_path_for_repo_path(project_root);
                let receipt_path =
                    pm_registry::pm_delivery_receipts_path_for_repo_path(project_root);
                let exact_acknowledgement = pm_wake_submit_is_acknowledged(
                    &loop_path,
                    &receipt_path,
                    &pending.decision.operation_id,
                    &pending.decision.registration.session_id,
                    &pending.decision.body_sha256,
                )
                .unwrap_or(false);
                if !exact_acknowledgement {
                    return true;
                }
                if let Some(current) = self.pending_pm_wake_deliveries.get_mut(project_root) {
                    current.phase = PmWakeDeliveryPhase::InFlight;
                }
                if let Err(error) =
                    self.start_pm_wake_delivery(pending.decision.clone(), false, true)
                {
                    self.handle_pm_wake_delivery_complete(
                        project_root,
                        &pending.decision.operation_id,
                        PmWakeDeliveryResult::CommitRetryable { error },
                    );
                }
                true
            }
            PmWakeDeliveryPhase::AwaitingCommitRetry => {
                if let Some(current) = self.pending_pm_wake_deliveries.get_mut(project_root) {
                    current.phase = PmWakeDeliveryPhase::InFlight;
                }
                if let Err(error) =
                    self.start_pm_wake_delivery(pending.decision.clone(), false, true)
                {
                    self.handle_pm_wake_delivery_complete(
                        project_root,
                        &pending.decision.operation_id,
                        PmWakeDeliveryResult::CommitRetryable { error },
                    );
                }
                true
            }
        }
    }

    fn start_pm_wake_delivery(
        &mut self,
        decision: PmWakeDecision,
        submit_only: bool,
        commit_only: bool,
    ) -> Result<(), String> {
        let expected_pty = if commit_only {
            None
        } else if submit_only {
            self.pending_pm_wake_deliveries
                .get(&decision.project_root)
                .filter(|pending| pending.decision.operation_id == decision.operation_id)
                .and_then(|pending| pending.expected_pty.clone())
                .ok_or_else(|| "PM wake CR retry lost its original PTY generation".to_string())
                .map(Some)?
        } else {
            let writer = current_pty_writer(&self.pty_writers, &decision.window_id)?;
            let pending = self
                .pending_pm_wake_deliveries
                .get_mut(&decision.project_root)
                .filter(|pending| pending.decision.operation_id == decision.operation_id)
                .ok_or_else(|| "PM wake reservation is no longer current".to_string())?;
            pending.expected_pty = Some(Arc::clone(&writer));
            Some(writer)
        };
        let pty_writers = Arc::clone(&self.pty_writers);
        let proxy = self.proxy.clone();
        let project_root = decision.project_root.clone();
        let operation_id = decision.operation_id.clone();
        let blocking_tasks = self.blocking_tasks.clone();
        blocking_tasks.try_spawn(move || {
            let result = run_pm_wake_delivery(
                &pty_writers,
                expected_pty.as_ref(),
                &decision,
                submit_only,
                commit_only,
            );
            proxy.send(UserEvent::PmWakeDeliveryComplete {
                project_root,
                operation_id,
                result,
            });
        })
    }

    pub(crate) fn handle_pm_wake_delivery_complete(
        &mut self,
        project_root: &Path,
        operation_id: &str,
        result: PmWakeDeliveryResult,
    ) -> Vec<OutboundEvent> {
        let Some(pending) = self.pending_pm_wake_deliveries.get(project_root).cloned() else {
            return Vec::new();
        };
        if pending.decision.operation_id != operation_id {
            return Vec::new();
        }
        match result {
            PmWakeDeliveryResult::Verified => {
                if let PmWakeKind::Delta { signals } = pending.decision.kind {
                    self.pm_wake_seen
                        .insert(project_root.to_path_buf(), signals);
                }
                self.pending_pm_wake_deliveries.remove(project_root);
                release_pm_submit(
                    &self.pm_submit_reservations,
                    &pending.decision.window_id,
                    &pending.decision.operation_id,
                );
                tracing::info!(
                    window_id = %pending.decision.window_id,
                    operation_id,
                    "resident PM wake submit verified and committed"
                );
            }
            PmWakeDeliveryResult::SubmitUnverified => {
                if let Some(current) = self.pending_pm_wake_deliveries.get_mut(project_root) {
                    current.phase = PmWakeDeliveryPhase::NeedsRecovery;
                }
                tracing::warn!(
                    window_id = %pending.decision.window_id,
                    operation_id,
                    "resident PM wake submit was not verified after bounded CR-only retry; retaining the reservation for manual recovery or pane restart"
                );
            }
            PmWakeDeliveryResult::CommitRetryable { error } => {
                if let Some(current) = self.pending_pm_wake_deliveries.get_mut(project_root) {
                    current.phase = PmWakeDeliveryPhase::AwaitingCommitRetry;
                }
                tracing::warn!(
                    %error,
                    operation_id,
                    "resident PM wake was submitted; next tick will retry its durable clock commit"
                );
            }
            PmWakeDeliveryResult::Terminal { error } => {
                self.pending_pm_wake_deliveries.remove(project_root);
                let loop_path = pm_registry::pm_loop_state_path_for_repo_path(project_root);
                if let Err(clear_error) = pm_registry::clear_pm_wake_delivery(
                    &loop_path,
                    &pending.decision.operation_id,
                    &pending.decision.window_id,
                ) {
                    tracing::warn!(%clear_error, operation_id, "failed to clear terminal PM wake reservation");
                }
                release_pm_submit(
                    &self.pm_submit_reservations,
                    &pending.decision.window_id,
                    &pending.decision.operation_id,
                );
                tracing::warn!(
                    %error,
                    window_id = %pending.decision.window_id,
                    operation_id,
                    "resident PM wake delivery ended before verification"
                );
            }
        }
        Vec::new()
    }

    pub(crate) fn clear_pm_submit_state_for_pty_generation_change(&mut self, window_id: &str) {
        if let Ok(mut reservations) = self.pm_submit_reservations.lock() {
            reservations.remove(window_id);
        }
        let mut project_roots = self
            .pending_pm_wake_deliveries
            .iter()
            .filter(|(_, pending)| pending.decision.window_id == window_id)
            .map(|(project_root, _)| project_root.clone())
            .collect::<HashSet<_>>();
        project_roots.extend(
            self.tabs
                .iter()
                .filter(|tab| {
                    tab.workspace.persisted().windows.iter().any(|window| {
                        crate::runtime_support::combined_window_id(&tab.id, &window.id) == window_id
                    })
                })
                .map(|tab| tab.project_root.clone()),
        );
        self.pending_pm_wake_deliveries
            .retain(|_, pending| pending.decision.window_id != window_id);
        if project_roots.is_empty() {
            return;
        }
        let window_id = window_id.to_string();
        let worker_window_id = window_id.clone();
        let spawn = self.blocking_tasks.try_spawn(move || {
            Self::reconcile_pm_wake_generation_change(project_roots, &worker_window_id);
        });
        if let Err(error) = spawn {
            // Leaving the old durable fence intact is fail-closed: no new
            // wake body can be staged until a later generation change or
            // explicit recovery gets a worker. The tao thread never falls
            // back to cross-process locking or fsync.
            tracing::warn!(%error, window_id, "failed to schedule PM wake generation reconciliation");
        }
    }

    fn reconcile_pm_wake_generation_change(project_roots: HashSet<PathBuf>, window_id: &str) {
        for project_root in project_roots {
            let loop_path = pm_registry::pm_loop_state_path_for_repo_path(&project_root);
            let exact_verified_wake_committed = pm_registry::load_pm_loop_state(&loop_path)
                .ok()
                .filter(|state| state.pending_delivery_window_id.as_deref() == Some(window_id))
                .is_some_and(|state| {
                    reconcile_crash_surviving_pm_wake(&project_root, &loop_path, &state)
                });
            if !exact_verified_wake_committed {
                if let Err(error) =
                    pm_registry::clear_pm_wake_delivery_for_window(&loop_path, window_id)
                {
                    tracing::warn!(%error, window_id, "failed to clear PM wake reservation after PTY generation change");
                }
            }
        }
    }

    /// SPEC-3431 FR-111 (T-206): PM-privileged message delivery into an
    /// agent pane. The general SPEC-3050 self-only contract is untouched —
    /// this separate path exists only for the live registered PM, and the
    /// principal is re-verified here, immediately before the injection:
    /// the presented session must be the durable `pm.json` registration of
    /// the project that owns the target window, and that registration's own
    /// pane must be live right now. Everything else is refused with a typed
    /// reply and no write.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn authenticated_pm_pane_send_input_events(
        &mut self,
        issuer: &AgentCapabilityIssuer,
        client_id: ClientId,
        grant: AgentCapabilityGrant,
        operation_id: &str,
        window_id: &str,
        text: &str,
        responder: Option<AgentPmSendResponder>,
    ) -> Vec<OutboundEvent> {
        let refuse = |error: String| {
            let event = BackendEvent::PaneSendResult {
                ok: false,
                window_id: Some(window_id.to_string()),
                error: Some(error),
            };
            if let Some(responder) = responder.as_ref() {
                let _ = responder.send(event);
                Vec::new()
            } else {
                vec![OutboundEvent::reply(client_id.clone(), event)]
            }
        };
        let canonical_operation_id = Uuid::parse_str(operation_id)
            .ok()
            .filter(|parsed| parsed.hyphenated().to_string() == operation_id)
            .map(|_| operation_id.to_string());
        let operation_id = match canonical_operation_id {
            Some(operation_id) => operation_id,
            None => {
                return refuse("pm pane send: operation_id must be a canonical UUID".to_string())
            }
        };
        let canonical_text = match canonical_pm_message_text(text) {
            Ok(text) => text,
            Err(error) => return refuse(error),
        };
        let body = canonical_text
            .strip_suffix('\r')
            .expect("canonical PM message always has one submit terminator");
        let body_sha256 = pm_registry::pm_delivery_prompt_sha256(body);
        let principal_session_id = grant.principal().session_id().to_string();
        let principal_receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(
            grant.principal().canonical_project_root(),
        );
        // Unit-level callers intentionally run synchronously. Production has
        // an origin responder and performs this cross-process receipt lookup
        // only after dispatching to the blocking worker.
        if responder.is_none() {
            match reconcile_pm_delivery_replay(
                &principal_receipt_path,
                &operation_id,
                &principal_session_id,
                window_id,
                &body_sha256,
            ) {
                Ok(PmDeliveryReplayOutcome::Missing) => {}
                Ok(PmDeliveryReplayOutcome::Verified) => {
                    release_pm_submit(&self.pm_submit_reservations, window_id, &operation_id);
                    return vec![OutboundEvent::reply(
                        client_id,
                        pm_delivery_replay_event(PmDeliveryReplayOutcome::Verified, String::new()),
                    )];
                }
                Ok(PmDeliveryReplayOutcome::Refused(error)) => return refuse(error),
                // A lookup error is not evidence that the operation exists.
                // The normal worker still attempts the write-ahead Prepared
                // commit, which fails before any PTY byte and reports the
                // durable-store error precisely.
                Err(_) => {}
            }
        }
        // The window's owning tab decides which project's PM registration is
        // the authority — a PM can never reach a pane of another project.
        // Project membership alone is insufficient: Shell and terminal agent
        // panes can also own PTYs, so accepting either would turn this
        // privileged instruction path into a command-injection surface.
        let Some((project_root, target_tab_id, target_preset, target_status, target_session_id)) =
            self.tabs.iter().find_map(|tab| {
                tab.workspace.persisted().windows.iter().find_map(|window| {
                    (crate::runtime_support::combined_window_id(&tab.id, &window.id) == window_id)
                        .then(|| {
                            (
                                tab.project_root.clone(),
                                tab.id.clone(),
                                window.preset,
                                window.status,
                                window.session_id.clone(),
                            )
                        })
                })
            })
        else {
            let error = format!(
                "pm pane send: unknown pane {window_id} (FR-111 refuses cross-project delivery)"
            );
            if let Some(responder) = responder.clone() {
                let spawn = defer_pm_delivery_replay_or_refusal(
                    &self.blocking_tasks,
                    principal_receipt_path,
                    operation_id,
                    principal_session_id,
                    window_id.to_string(),
                    body_sha256,
                    Arc::clone(&self.pm_submit_reservations),
                    responder,
                    error.clone(),
                );
                return match spawn {
                    Ok(()) => Vec::new(),
                    Err(spawn_error) => {
                        refuse(format!("pm pane send: worker unavailable: {spawn_error}"))
                    }
                };
            }
            return refuse(error);
        };
        let target_is_live_agent = matches!(
            target_preset,
            WindowPreset::Agent | WindowPreset::Claude | WindowPreset::Codex
        ) && matches!(
            target_status,
            WindowProcessStatus::Running | WindowProcessStatus::Idle | WindowProcessStatus::Waiting
        ) && target_session_id.as_deref().is_some_and(
            |target_session_id| {
                self.active_agent_sessions
                    .get(window_id)
                    .is_some_and(|session| {
                        session.window_id == window_id
                            && session.tab_id == target_tab_id
                            && session.session_id == target_session_id
                    })
            },
        ) && matches!(
            self.window_status(window_id),
            Some(
                WindowProcessStatus::Running
                    | WindowProcessStatus::Idle
                    | WindowProcessStatus::Waiting
            )
        );
        if !target_is_live_agent {
            let error = format!("pm pane send: target {window_id} is not a live agent pane");
            if let Some(responder) = responder.clone() {
                let spawn = defer_pm_delivery_replay_or_refusal(
                    &self.blocking_tasks,
                    principal_receipt_path,
                    operation_id,
                    principal_session_id,
                    window_id.to_string(),
                    body_sha256,
                    Arc::clone(&self.pm_submit_reservations),
                    responder,
                    error.clone(),
                );
                return match spawn {
                    Ok(()) => Vec::new(),
                    Err(spawn_error) => {
                        refuse(format!("pm pane send: worker unavailable: {spawn_error}"))
                    }
                };
            }
            return refuse(error);
        }
        let target_session_id = target_session_id.expect("live target requires a Session id");
        let canonical_project_root = dunce::canonicalize(&project_root)
            .map(|path| gwt_core::paths::normalize_windows_child_process_path(&path));
        if canonical_project_root.as_ref().ok().map(PathBuf::as_path)
            != Some(grant.principal().canonical_project_root())
        {
            let error =
                "pm pane send: authenticated principal does not own the target project".to_string();
            if let Some(responder) = responder.clone() {
                let spawn = defer_pm_delivery_replay_or_refusal(
                    &self.blocking_tasks,
                    principal_receipt_path,
                    operation_id,
                    principal_session_id,
                    window_id.to_string(),
                    body_sha256,
                    Arc::clone(&self.pm_submit_reservations),
                    responder,
                    error.clone(),
                );
                return match spawn {
                    Ok(()) => Vec::new(),
                    Err(spawn_error) => {
                        refuse(format!("pm pane send: worker unavailable: {spawn_error}"))
                    }
                };
            }
            return refuse(error);
        }
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
        let receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(&project_root);
        let text = canonical_text;
        let principal_session_is_live = self.live_pm_window_id(&principal_session_id).is_some();
        // Direct unit callers retain synchronous refusal semantics. Production
        // always carries an origin responder, so filesystem/PTY preflight is
        // moved to the blocking worker and can never stall the tao thread.
        let prepared = if responder.is_none() {
            match prepare_pm_send_context(
                &prefs_path,
                &principal_session_id,
                principal_session_is_live,
                &self.pty_writers,
                window_id,
            ) {
                Ok(prepared) => Some(prepared),
                Err(error) => return refuse(error),
            }
        } else {
            None
        };
        let body = text
            .strip_suffix('\r')
            .expect("canonical PM message always has one submit terminator");
        let text = protected_delivery_prompt(&operation_id, body);
        match reserve_pm_submit(&self.pm_submit_reservations, window_id, &operation_id) {
            Ok(true) => {}
            Ok(false) => {
                return refuse(format!(
                    "pm pane send: pane {window_id} already has a privileged submit in progress"
                ))
            }
            Err(error) => return refuse(error),
        }
        let issuer = issuer.clone();
        let writers = Arc::clone(&self.pty_writers);
        let reservations = Arc::clone(&self.pm_submit_reservations);
        let proxy = self.proxy.clone();
        let target_window_id = window_id.to_string();
        let reservation_operation_id = operation_id.clone();
        let worker_responder = responder.clone();
        let worker_client_id = client_id.clone();
        let blocking_tasks = self.blocking_tasks.clone();
        let spawn = blocking_tasks.try_spawn(move || {
            let responder = worker_responder;
            let _operation_deadline = responder.as_ref().map(|responder| {
                gwt_core::operation_deadline::ScopedOperationDeadline::enter(responder.deadline())
            });
            let mut writes = 0_usize;
            let mut body_was_attempted = false;
            let mut receipt_prepared = false;
            let mut durable_ambiguous = false;
            let body_sha256 = body_sha256;
            let prepared_receipt = pm_registry::PmDeliveryReceipt {
                operation_id: operation_id.clone(),
                recorded_at: chrono::Utc::now()
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                status: pm_registry::PmDeliveryReceiptStatus::Prepared,
                principal_session_id: principal_session_id.clone(),
                target_window_id: target_window_id.clone(),
                target_session_id: target_session_id.clone(),
                body_sha256: body_sha256.clone(),
                error_code: None,
            };
            let prepare_outcome = pm_registry::prepare_pm_delivery_receipt(
                &receipt_path,
                &prepared_receipt,
            )
            .map_err(|error| format!("PM delivery receipt Prepared commit failed: {error}"));
            let result = match prepare_outcome {
                Ok(pm_registry::PmDeliveryPrepareOutcome::Prepared) => {
                    receipt_prepared = true;
                    let context = prepared.map_or_else(
                        || {
                            prepare_pm_send_context(
                                &prefs_path,
                                &principal_session_id,
                                principal_session_is_live,
                                &writers,
                                &target_window_id,
                            )
                        },
                        Ok,
                    );
                    context.and_then(|(registration, expected_pty)| {
                let transaction = Arc::clone(&expected_pty)
                    .reserve_input_transaction()
                    .map_err(|error| format!("PM pane input transaction unavailable: {error}"))?;
                super::pty_io::drive_verified_pane_submit(
                    &text,
                    PM_WAKE_SUBMIT_ATTEMPTS,
                    |bytes| {
                        let _capability_lease = issuer
                            .try_begin_current_grant_mutation(&grant)
                            .ok_or_else(|| {
                                "PM capability changed before input mutation".to_string()
                            })?;
                        write_registered_pm_pty(
                            &writers,
                            &target_window_id,
                            &expected_pty,
                            &prefs_path,
                            &registration,
                            bytes,
                            responder.as_ref(),
                            |bytes| {
                                if bytes != b"\r" {
                                    body_was_attempted = true;
                                }
                                transaction
                                    .write_input(bytes)
                                    .map_err(|error| error.to_string())
                            },
                        )?;
                        if writes == 0 {
                            invoke_pm_pane_after_body_test_hook();
                        }
                        writes = writes.saturating_add(1);
                        Ok(())
                    },
                    thread::sleep,
                    || pm_pane_submit_is_acknowledged(&receipt_path, &operation_id),
                )
                    })
                }
                Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                    pm_registry::PmDeliveryReceiptStatus::Verified,
                )) => Ok(super::pty_io::VerifiedPaneSubmitOutcome::Verified {
                    submit_attempts: 0,
                }),
                Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                    pm_registry::PmDeliveryReceiptStatus::Refused,
                )) => Err("PM delivery operation was already refused".to_string()),
                Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                    pm_registry::PmDeliveryReceiptStatus::Prepared,
                )) => {
                    receipt_prepared = true;
                    durable_ambiguous = true;
                    Err("PM delivery operation may already have staged its prompt body".to_string())
                }
                Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                    pm_registry::PmDeliveryReceiptStatus::Ambiguous,
                )) => {
                    durable_ambiguous = true;
                    Err("PM delivery operation may already have staged its prompt body".to_string())
                }
                Err(error) => Err(error),
            };
            let result = match result {
                Ok(super::pty_io::VerifiedPaneSubmitOutcome::Unverified { .. }) => {
                    durable_ambiguous = true;
                    let terminal = receipt_prepared
                        .then(|| {
                            pm_registry::finish_pm_delivery_receipt(
                            &receipt_path,
                            &operation_id,
                            &target_session_id,
                            &body_sha256,
                            pm_registry::PmDeliveryReceiptStatus::Ambiguous,
                            Some("submit_unverified"),
                        )
                        })
                        .transpose();
                    match terminal {
                        Ok(Some(pm_registry::PmDeliveryReceiptStatus::Verified)) => Ok(
                            super::pty_io::VerifiedPaneSubmitOutcome::Verified {
                                submit_attempts: PM_WAKE_SUBMIT_ATTEMPTS,
                            },
                        ),
                        Err(error) => Err(format!(
                            "PM pane submit was not verified; PM delivery receipt terminal commit failed: {error}"
                        )),
                        _ => Err("PM pane submit was not verified".to_string()),
                    }
                }
                Err(mut error) if body_was_attempted || durable_ambiguous => {
                    durable_ambiguous = true;
                    if receipt_prepared {
                        if let Err(receipt_error) = pm_registry::finish_pm_delivery_receipt(
                            &receipt_path,
                            &operation_id,
                            &target_session_id,
                            &body_sha256,
                            pm_registry::PmDeliveryReceiptStatus::Ambiguous,
                            Some("body_staged_or_unknown"),
                        ) {
                            error.push_str(&format!(
                                "; PM delivery receipt terminal commit failed: {receipt_error}"
                            ));
                        }
                    }
                    Err(error)
                }
                Err(mut error) => {
                    if receipt_prepared {
                        if let Err(receipt_error) = pm_registry::finish_pm_delivery_receipt(
                            &receipt_path,
                            &operation_id,
                            &target_session_id,
                            &body_sha256,
                            pm_registry::PmDeliveryReceiptStatus::Refused,
                            Some("mutation_refused"),
                        ) {
                            error.push_str(&format!(
                                "; PM delivery receipt terminal commit failed: {receipt_error}"
                            ));
                        }
                    }
                    Err(error)
                }
                verified => verified,
            };
            let result = if result.is_err()
                && pm_pane_submit_is_acknowledged(&receipt_path, &operation_id).unwrap_or(false)
            {
                durable_ambiguous = false;
                Ok(super::pty_io::VerifiedPaneSubmitOutcome::Verified {
                    submit_attempts: PM_WAKE_SUBMIT_ATTEMPTS,
                })
            } else {
                result
            };
            let ambiguous_body = durable_ambiguous || (result.is_err() && body_was_attempted);
            let event = match result {
                Ok(super::pty_io::VerifiedPaneSubmitOutcome::Verified { .. }) => {
                    BackendEvent::PaneSendResult {
                        ok: true,
                        window_id: None,
                        error: None,
                    }
                }
                Ok(super::pty_io::VerifiedPaneSubmitOutcome::Unverified { .. }) => {
                    BackendEvent::PaneSendResult {
                        ok: false,
                        window_id: None,
                        error: Some("PM pane submit was not verified".to_string()),
                    }
                }
                Err(error) if ambiguous_body => BackendEvent::PaneSendResult {
                    ok: false,
                    window_id: None,
                    error: Some(format!(
                        "{error}; prompt body may be staged — do not retry until the target pane restarts"
                    )),
                },
                Err(error) => BackendEvent::PaneSendResult {
                    ok: false,
                    window_id: None,
                    error: Some(error),
                },
            };
            if !ambiguous_body {
                release_pm_submit(&reservations, &target_window_id, &operation_id);
            }
            if let Some(responder) = responder {
                let _ = responder.send(event);
            } else {
                proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                    worker_client_id,
                    event,
                )]));
            }
        });
        if let Err(error) = spawn {
            release_pm_submit(
                &self.pm_submit_reservations,
                window_id,
                &reservation_operation_id,
            );
            return refuse(format!("pm pane send: worker unavailable: {error}"));
        }
        Vec::new()
    }

    /// Authoritative liveness for a stored PM registration (FR-001): the
    /// registered session must have a live pane right now.
    pub(crate) fn pm_registration_is_live(&self, registration: &PmRegistration) -> bool {
        self.live_pm_window_id(&registration.session_id).is_some()
    }

    /// Window id of the live pane bound to `session_id`, if any: the session
    /// is in the active registry, its window is still on the canvas, and the
    /// composed window status is not terminal.
    pub(crate) fn live_pm_window_id(&self, session_id: &str) -> Option<String> {
        self.active_agent_sessions
            .iter()
            .find_map(|(window_id, session)| {
                if session.session_id != session_id || !self.window_lookup.contains_key(window_id) {
                    return None;
                }
                match self.window_status(window_id) {
                    Some(WindowProcessStatus::Stopped) | Some(WindowProcessStatus::Error) => None,
                    _ => Some(window_id.clone()),
                }
            })
    }

    /// Resolve every fallible PM-registration input before a Work transition
    /// is staged. The actual singleton CAS remains a separate commit so the
    /// Workspace transaction can publish Resume only after both resources
    /// agree on the launch.
    pub(crate) fn prepare_pm_registration_after_launch(
        &self,
        project_root: &Path,
        session_id: &str,
        agent_id: &str,
        worktree_path: &Path,
    ) -> Result<PreparedPmRegistration, String> {
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let prefs = pm_registry::load_pm_registration_locked(&prefs_path)
            .map_err(|error| format!("PM registration preflight failed: {error}"))?;
        let worktree_path_text = worktree_path.to_string_lossy().into_owned();
        let exact_candidate = prefs.as_ref().is_some_and(|existing| {
            existing.session_id == session_id
                && existing.agent_id == agent_id
                && existing.worktree_path == worktree_path_text
        });
        if let Some(existing) = prefs
            .as_ref()
            .filter(|existing| !exact_candidate && self.pm_registration_is_live(existing))
        {
            tracing::warn!(
                existing_session = %existing.session_id,
                new_session = %session_id,
                "PM launch completed while another live PM is registered; keeping existing"
            );
            return Err("PM registration refused because another live PM owns the project".into());
        }
        let candidate = match prefs {
            Some(existing)
                if existing.session_id == session_id
                    && existing.agent_id == agent_id
                    && existing.worktree_path == worktree_path_text =>
            {
                existing
            }
            _ => PmRegistration {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                worktree_path: worktree_path_text,
                created_at: Some(
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                ),
                consecutive_crashes: 0,
                next_not_before: None,
            },
        };
        Ok(PreparedPmRegistration {
            project_root: project_root.to_path_buf(),
            work_event_root: worktree_path.to_path_buf(),
            prefs_path,
            candidate,
            commit_mode: PmRegistrationCommitMode::ReplaceStaleOrEmpty,
            operation_id: format!("pm-launch-registration-{session_id}"),
        })
    }

    /// Prepare succession only while the durable PM registration names the
    /// exact source Session (or already names the exact successor after a
    /// response-loss retry). The full predecessor snapshot becomes the CAS
    /// precondition used by the external commit.
    pub(crate) fn prepare_pm_succession_after_launch(
        &self,
        project_root: &Path,
        source_session_id: &str,
        successor_session_id: &str,
        agent_id: &str,
        worktree_path: &Path,
    ) -> Result<Option<PreparedPmRegistration>, String> {
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let Some(existing) = pm_registry::load_pm_registration_locked(&prefs_path)
            .map_err(|error| format!("PM succession preflight failed: {error}"))?
        else {
            return Ok(None);
        };
        let worktree_path_text = worktree_path.to_string_lossy().into_owned();
        let candidate = if existing.session_id == successor_session_id
            && existing.agent_id == agent_id
            && existing.worktree_path == worktree_path_text
        {
            existing.clone()
        } else if existing.session_id == source_session_id {
            PmRegistration {
                session_id: successor_session_id.to_string(),
                agent_id: agent_id.to_string(),
                worktree_path: worktree_path_text,
                created_at: Some(
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                ),
                consecutive_crashes: 0,
                next_not_before: None,
            }
        } else {
            return Ok(None);
        };
        Ok(Some(PreparedPmRegistration {
            project_root: project_root.to_path_buf(),
            work_event_root: worktree_path.to_path_buf(),
            prefs_path,
            candidate,
            commit_mode: PmRegistrationCommitMode::ReplaceExact(existing),
            operation_id: format!("pm-launch-registration-{successor_session_id}"),
        }))
    }

    /// The external logical commit paired with a staged Workspace Resume.
    /// It re-evaluates the actual registration under the PM lock so a PM that
    /// appeared after preflight can never be replaced.
    pub(crate) fn commit_prepared_pm_registration(
        &mut self,
        prepared: &PreparedPmRegistration,
    ) -> Result<(), String> {
        invoke_pm_registration_commit_hook()?;
        let candidate = prepared.candidate.clone();
        let prefs = match &prepared.commit_mode {
            PmRegistrationCommitMode::ReplaceStaleOrEmpty => {
                let (prefs, outcome) = pm_registry::try_register_pm(
                    &prepared.prefs_path,
                    candidate.clone(),
                    |existing| self.pm_registration_is_live(existing),
                )
                .map_err(|error| format!("PM registration commit failed: {error}"))?;
                if let pm_registry::PmRegisterOutcome::RejectedLive { existing } = outcome {
                    tracing::warn!(
                        existing_session = %existing.session_id,
                        new_session = %candidate.session_id,
                        "PM launch completed while another live PM is registered; keeping existing"
                    );
                    return Err(
                        "PM registration refused because another live PM owns the project".into(),
                    );
                }
                prefs
            }
            PmRegistrationCommitMode::ReplaceExact(expected) => {
                let (prefs, replaced) = pm_registry::compare_and_swap_pm_registration(
                    &prepared.prefs_path,
                    expected,
                    candidate.clone(),
                )
                .map_err(|error| format!("PM succession commit failed: {error}"))?;
                if !replaced {
                    return Err(
                        "PM registration changed before succession could commit".to_string()
                    );
                }
                prefs
            }
        };
        if prefs.registration.as_ref() != Some(&candidate) {
            return Err(
                "PM registration commit readback did not match the launched session".into(),
            );
        }
        Ok(())
    }

    /// Classify an ambiguous registration response from a locked durable
    /// readback. Volatile state is published only after the Workspace owner
    /// resolves the paired transaction.
    pub(crate) fn prepared_pm_registration_matches(
        &self,
        prepared: &PreparedPmRegistration,
    ) -> Result<bool, String> {
        let registration = pm_registry::load_pm_registration_locked(&prepared.prefs_path)
            .map_err(|error| format!("PM registration readback failed: {error}"))?;
        Ok(registration.as_ref() == Some(&prepared.candidate))
    }

    /// Publish the volatile PM marker only after the paired Workspace commit
    /// has become visible. The durable registration remains the authority.
    pub(crate) fn sync_prepared_pm_registration_cache(
        &mut self,
        prepared: &PreparedPmRegistration,
    ) {
        self.sync_pm_session_cache(&prepared.project_root, Some(&prepared.candidate));
    }

    /// Keep the per-broadcast PM marker in step with the durable record.
    fn sync_pm_session_cache(
        &mut self,
        project_root: &Path,
        registration: Option<&PmRegistration>,
    ) {
        match registration {
            Some(registration) => {
                self.pm_sessions
                    .insert(project_root.to_path_buf(), registration.session_id.clone());
            }
            None => {
                self.pm_sessions.remove(project_root);
            }
        }
    }

    /// FR-013: an explicit close of the PM pane clears the registration so
    /// nothing auto-restarts it. Settings (auto_start) survive. A clean PM
    /// worktree is reaped with the registration (T-016); local work keeps it
    /// for reuse by the next PM.
    ///
    /// Returns whether this close actually deregistered a PM, so the caller can
    /// refresh the settings panel only for the close that changed PM state.
    pub(super) fn deregister_pm_for_closed_window(
        &mut self,
        project_root: &Path,
        session_id: &str,
    ) -> bool {
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        match pm_registry::deregister_pm(&prefs_path, session_id) {
            Ok((_, true)) => {
                tracing::info!(%session_id, "PM pane closed; registration cleared");
                self.sync_pm_session_cache(project_root, None);
                Self::cleanup_pm_worktree(project_root);
                true
            }
            Ok((_, false)) => false,
            Err(error) => {
                tracing::warn!(%error, "failed to deregister PM on window close");
                false
            }
        }
    }

    /// T-016: remove the canonical PM worktree when it holds no local work.
    /// Only the canonical `<gwt_project_dir>/pm/worktree` location is ever
    /// touched — a corrupted registration cannot direct the reaper at an
    /// arbitrary path. Fail-closed: any uncertainty keeps the worktree.
    fn cleanup_pm_worktree(project_root: &Path) {
        let worktree = pm_registry::pm_worktree_path_for_repo_path(project_root);
        if !worktree.exists() {
            return;
        }
        let manager = gwt_git::WorktreeManager::new(Self::pm_git_root(project_root));
        match manager.ephemeral_worktree_has_local_work(&worktree) {
            Ok(false) => {
                if let Err(error) = manager.remove_force_twice(&worktree) {
                    tracing::warn!(%error, "failed to remove the clean PM worktree");
                }
            }
            Ok(true) => {
                tracing::info!(
                    worktree = %worktree.display(),
                    "PM worktree has local work; keeping it for reuse"
                );
            }
            Err(error) => {
                tracing::warn!(%error, "PM worktree local-work check failed; keeping it");
            }
        }
    }

    /// FR-003: an unexpected exit of the registered PM records one crash on
    /// the backoff ladder and, when the ladder allows, respawns immediately
    /// by re-running the ensure gate (which resumes the same conversation).
    pub(super) fn handle_pm_crash(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        session_id: &str,
    ) -> Vec<OutboundEvent> {
        let prefs_path = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let respawn_now = match pm_registry::mutate_pm_prefs(&prefs_path, |prefs| {
            let registration = prefs.registration.as_mut()?;
            if registration.session_id != session_id {
                return None;
            }
            Some(pm_registry::apply_pm_crash_backoff(registration, &now))
        }) {
            Ok((_, Some(respawn_now))) => respawn_now,
            Ok((_, None)) => return Vec::new(),
            Err(error) => {
                tracing::warn!(%error, "failed to record PM crash backoff");
                return Vec::new();
            }
        };
        if !respawn_now {
            return Vec::new();
        }
        self.ensure_pm_agent_for_tab(tab_id, PmEnsureTrigger::Explicit)
    }

    fn pm_window_ids(&self, tab_id: &str) -> HashSet<String> {
        self.tab(tab_id)
            .map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .map(|window| window.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Track windows added by a PM spawn so launch completion can write the
    /// registration (keyed by combined window id, mapped to project root).
    fn mark_new_pm_windows(&mut self, tab_id: &str, before: &HashSet<String>, project_root: &Path) {
        let new_ids: Vec<String> = self
            .tab(tab_id)
            .map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .filter(|window| !before.contains(&window.id))
                    .map(|window| crate::runtime_support::combined_window_id(tab_id, &window.id))
                    .collect()
            })
            .unwrap_or_default();
        for combined in new_ids {
            self.pending_pm_launches
                .insert(combined, project_root.to_path_buf());
        }
    }

    fn spawn_pm_agent(&mut self, tab_id: &str, project_root: &Path) -> Vec<OutboundEvent> {
        let profile = pm_registry::pm_prefs_path_for_repo_path(project_root);
        let profile = pm_registry::load_pm_prefs(&profile)
            .map(|prefs| prefs.settings.launch_profile_or_default())
            .unwrap_or_else(|_| pm_registry::PmLaunchProfile::default_profile());
        let worktree = match Self::ensure_pm_worktree(project_root) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    project_root = %project_root.display(),
                    error,
                    "failed to prepare the PM worktree; PM not started"
                );
                return Vec::new();
            }
        };
        // T-052: the `$gwt-pm` bootstrap prompt resolves against the guidance
        // skill that managed-asset materialization writes into this worktree.
        // Writing it here instead would be futile — the launch's own asset
        // refresh prunes unbundled `gwt-*` skills right after.
        let config = Self::pm_launch_config(&worktree, &profile);
        let before = self.pm_window_ids(tab_id);
        match self.spawn_agent_window_at_geometry(tab_id, config, PM_WINDOW_GEOMETRY, None, None) {
            Ok(events) => {
                self.mark_new_pm_windows(tab_id, &before, project_root);
                events
            }
            Err(error) => {
                tracing::warn!(error, "PM silent spawn failed");
                Vec::new()
            }
        }
    }

    /// SPEC-3431 FR-026: the launch config for a fresh PM spawn.
    ///
    /// Pure so the resolved agent/model can be asserted without spawning.
    /// `suppress_execution_control` is set because the PM is a conversational
    /// role, not an execution-controlled implementation session.
    ///
    /// Permissions are always skipped (FR-012, user ruling 2026-08-06). The PM
    /// subscribes, reconciles, registers Issues, and instructs launches with no
    /// user present, so a permission prompt in that loop is a deadlock nobody
    /// is watching. Same reasoning as the Issue Monitor's
    /// `force_skip_permissions_for_autonomous`, and likewise not a per-project
    /// choice — a PM that can be configured into a hang will eventually hang.
    pub(crate) fn pm_launch_config(
        worktree: &Path,
        profile: &pm_registry::PmLaunchProfile,
    ) -> gwt_agent::LaunchConfig {
        let agent_id = gwt_agent::resolve_agent_id(&profile.agent_id)
            .unwrap_or(gwt_agent::AgentId::ClaudeCode);
        let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id)
            .working_dir(worktree.to_path_buf())
            .skip_permissions(true)
            .extra_arg(PM_BOOTSTRAP_PROMPT);
        if let Some(model) = profile.model.as_deref().filter(|value| !value.is_empty()) {
            builder = builder.model(model);
        }
        if let Some(reasoning) = profile
            .reasoning
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            builder = builder.reasoning_level(reasoning);
        }
        if let Some(version) = profile.version.as_deref().filter(|value| !value.is_empty()) {
            builder = builder.version(version);
        }
        let mut config = builder.build();
        config.suppress_execution_control = true;
        config
    }

    /// Dedicated detached worktree for the PM session (research R-10). Its
    /// lifecycle is bound to the PM registration; T-016 adds GC.
    fn ensure_pm_worktree(project_root: &Path) -> Result<PathBuf, String> {
        let path = pm_registry::pm_worktree_path_for_repo_path(project_root);
        if path.join(".git").exists() {
            return Ok(path);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let manager = gwt_git::WorktreeManager::new(Self::pm_git_root(project_root));
        manager
            .create_detached("HEAD", &path)
            .map_err(|error| error.to_string())?;
        Ok(path)
    }

    /// Issue #3497: the opened project root is not always a repository — the
    /// bare layout (`parent/` holding `parent/<name>.git` plus worktrees) is
    /// exactly what the launch paths already resolve through
    /// `main_worktree_root`. Every PM `WorktreeManager` goes through the same
    /// resolution, or `git worktree` dies with "not a git repository" and the
    /// PM silently never comes up.
    fn pm_git_root(project_root: &Path) -> PathBuf {
        gwt_git::worktree::main_worktree_root(project_root)
            .unwrap_or_else(|_| project_root.to_path_buf())
    }
}

/// Test-only gate: the PM ensure is opt-in per test thread. Without it every
/// pre-existing startup/restore test that drives bootstrap or project-open
/// would spawn a PM pane — and its async launch thread — polluting session
/// and window counts across the suite. Production builds compile this out.
#[cfg(test)]
pub(crate) mod test_gate {
    use std::cell::Cell;

    thread_local! {
        pub(crate) static PM_ENSURE_ENABLED: Cell<bool> = const { Cell::new(false) };
    }

    /// RAII enable for PM ensure in a test; resets on drop.
    pub(crate) struct PmEnsureTestGuard;

    impl PmEnsureTestGuard {
        pub(crate) fn enable() -> Self {
            PM_ENSURE_ENABLED.with(|cell| cell.set(true));
            PmEnsureTestGuard
        }
    }

    impl Drop for PmEnsureTestGuard {
        fn drop(&mut self) {
            PM_ENSURE_ENABLED.with(|cell| cell.set(false));
        }
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::commit_verified_pm_wake;

    #[test]
    fn wake_clock_commit_requires_the_exact_submitted_acknowledgement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state_path = temp.path().join("project-state/pm-loop.json");
        gwt::pm_registry::save_pm_loop_state(
            &state_path,
            &gwt::pm_registry::PmLoopState {
                consecutive_continuations: 7,
                last_continued_at: Some("2026-08-11T00:00:00Z".to_string()),
                last_user_prompt_at: Some("2026-08-11T00:00:01Z".to_string()),
                pending_own_block: true,
                last_wake_at: None,
                last_delivery_operation_id: Some(
                    "72fc3cd4-ad49-43e3-bf3d-d791357643a3".to_string(),
                ),
                last_delivery_submitted_at: None,
                ..gwt::pm_registry::PmLoopState::default()
            },
        )
        .expect("seed incomplete acknowledgement");

        let receipt_path = temp.path().join("project-state/pm-delivery-receipts.jsonl");
        let error = commit_verified_pm_wake(
            &state_path,
            &receipt_path,
            "72fc3cd4-ad49-43e3-bf3d-d791357643a3",
            "pm-session",
            "",
        )
        .expect_err("an operation id without a submit timestamp is not verified");
        assert!(error.contains("acknowledgement"));
        let state = gwt::pm_registry::load_pm_loop_state(&state_path).expect("state after refusal");
        assert_eq!(state.consecutive_continuations, 7);
        assert!(state.pending_own_block);
        assert_eq!(state.last_wake_at, None);
    }

    #[test]
    fn exact_submitted_ack_commits_only_the_wake_budget_and_clock() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state_path = temp.path().join("project-state/pm-loop.json");
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643a3";
        gwt::pm_registry::save_pm_loop_state(
            &state_path,
            &gwt::pm_registry::PmLoopState {
                consecutive_continuations: 7,
                last_continued_at: Some("2026-08-11T00:00:00Z".to_string()),
                last_user_prompt_at: Some("2026-08-11T00:00:01Z".to_string()),
                pending_own_block: true,
                last_wake_at: None,
                last_delivery_operation_id: Some(operation_id.to_string()),
                last_delivery_submitted_at: Some("2026-08-11T00:00:02Z".to_string()),
                ..gwt::pm_registry::PmLoopState::default()
            },
        )
        .expect("seed acknowledgement");

        let receipt_path = temp.path().join("project-state/pm-delivery-receipts.jsonl");
        commit_verified_pm_wake(&state_path, &receipt_path, operation_id, "pm-session", "")
            .expect("commit verified wake");

        let state = gwt::pm_registry::load_pm_loop_state(&state_path).expect("committed state");
        assert_eq!(state.consecutive_continuations, 0);
        assert!(!state.pending_own_block);
        assert_eq!(state.last_wake_at.as_deref(), Some("2026-08-11T00:00:02Z"));
        assert_eq!(
            state.last_continued_at.as_deref(),
            Some("2026-08-11T00:00:00Z")
        );
        assert_eq!(
            state.last_user_prompt_at.as_deref(),
            Some("2026-08-11T00:00:01Z")
        );
    }
}
