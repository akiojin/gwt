//! Agent pane operations execute against the fixed project's captured runtime.
use crate::{
    agent_capability::{
        AgentCapabilityGrant, AgentCapabilityIssuer, AgentPmSendResponder, AgentSessionPrincipal,
    },
    project_transport::OutboundEvent,
    BackendEvent, ProjectTabView,
};
use base64::Engine as _;
use gwt_terminal::{Pane, PtyHandle};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    thread,
    time::{Duration, Instant},
};

pub struct PtyWriterEntry {
    pub project_key: gwt_core::repo_hash::ProjectKey,
    pub handle: Arc<PtyHandle>,
    pub current: Arc<std::sync::atomic::AtomicBool>,
    pub monitor_runtime: Option<crate::monitor_duplicate_runtime::MonitorRuntimeRegistration>,
    // Only the latest fully written PM prompt in this exact PTY generation
    // may receive one bounded submit-only recovery. Never restore from disk.
    pub pm_delivery_replay: Mutex<Option<PmDeliveryReplayGuard>>,
}

pub struct PmDeliveryReplayGuard {
    pub operation_id: String,
    pub target_session_id: String,
    pub ordinary_revision: u64,
    pub transaction_revision: u64,
}

pub type PtyWriterRegistry = Arc<RwLock<HashMap<String, Arc<PtyWriterEntry>>>>;

const PANE_SUBMIT_SETTLE: Duration = Duration::from_millis(400);

/// Split one pane payload into the body and its submit terminator. Input that
/// carries no terminator is not a submit and is returned whole, so raw
/// keystroke forwarding stays byte-exact.
pub fn split_pane_submit(text: &str) -> (&str, Option<&str>) {
    for terminator in ["\r\n", "\r", "\n"] {
        if let Some(body) = text.strip_suffix(terminator) {
            return (body, Some(terminator));
        }
    }
    (text, None)
}

/// Issue #4909: the body of one injected prompt as the PTY must receive it.
///
/// With bracketed-paste mode on (Claude Code and Codex both enable it), the
/// body is delivered as one paste, `ESC [ 200 ~ … ESC [ 201 ~`, so the
/// composer takes `@`, a leading `/`, `?`, and embedded newlines as pasted
/// text instead of as the keys that open its file / slash-command pickers.
/// The submit terminator is deliberately not part of the paste: it is still
/// written on its own after the TUI has settled (SPEC-3431 FR-108c), so the
/// provider's UserPromptSubmit hook sees exactly one prompt. With the mode
/// off, the body is written raw, exactly as before.
pub fn pane_input_body_bytes(body: &str, bracketed_paste: bool) -> Result<Vec<u8>, String> {
    // Issue #5839: Windows Codex receives console key events even inside a
    // bracketed paste. Ctrl+T / F3 (ESC O R) can therefore open Transcript/Find.
    if let Some(byte) = body
        .bytes()
        .find(|byte| byte.is_ascii_control() && !matches!(byte, b'\t' | b'\r' | b'\n'))
    {
        return Err(format!(
            "prompt body contains terminal control byte 0x{byte:02x}; remove it before sending"
        ));
    }
    if !bracketed_paste {
        return Ok(body.as_bytes().to_vec());
    }
    let mut bytes = Vec::with_capacity(body.len() + 12);
    bytes.extend_from_slice(BRACKETED_PASTE_START);
    bytes.extend_from_slice(body.as_bytes());
    bytes.extend_from_slice(BRACKETED_PASTE_END);
    Ok(bytes)
}

const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

/// Result of a body-once, submit-until-verified delivery. Verification is a
/// semantic target acknowledgement, never merely a successful PTY write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifiedPaneSubmitOutcome {
    Verified { submit_attempts: usize },
    Unverified { submit_attempts: usize },
}

/// Write the prompt body once, then retry only its submit terminator until the
/// injected semantic acknowledgement says the exact target turn started.
pub fn drive_verified_pane_submit(
    text: &str,
    bracketed_paste: bool,
    max_submit_attempts: usize,
    mut write: impl FnMut(&[u8]) -> Result<(), String>,
    mut settle: impl FnMut(Duration),
    mut is_verified: impl FnMut() -> Result<bool, String>,
) -> Result<VerifiedPaneSubmitOutcome, String> {
    let (body, submit) = split_pane_submit(text);
    if !body.is_empty() {
        write(&pane_input_body_bytes(body, bracketed_paste)?)?;
    }
    let Some(submit) = submit else {
        return Ok(VerifiedPaneSubmitOutcome::Verified { submit_attempts: 0 });
    };

    // Give the TUI composer time to consume the body before the first submit.
    settle(PANE_SUBMIT_SETTLE);
    if body.is_empty() && is_verified()? {
        return Ok(VerifiedPaneSubmitOutcome::Verified { submit_attempts: 0 });
    }
    for submit_attempts in 1..=max_submit_attempts {
        if submit_attempts > 1 && is_verified()? {
            return Ok(VerifiedPaneSubmitOutcome::Verified {
                submit_attempts: submit_attempts - 1,
            });
        }
        write(submit.as_bytes())?;
        // Every submit, including the final attempt, receives a full semantic
        // ACK grace period before the operation can become Ambiguous.
        settle(PANE_SUBMIT_SETTLE);
        if is_verified()? {
            return Ok(VerifiedPaneSubmitOutcome::Verified { submit_attempts });
        }
    }
    Ok(VerifiedPaneSubmitOutcome::Unverified {
        submit_attempts: max_submit_attempts,
    })
}

/// Write one pane payload as the TUIs expect it: the body first, then the
/// submit byte on its own once the TUI has settled (SPEC-3431 FR-108c). The
/// delay runs on a detached thread holding an `Arc` clone of the pane, so the
/// event loop is never blocked and every other pane keeps streaming.
pub fn write_pane_input_then_submit(pane: &Arc<Mutex<Pane>>, text: &str) -> Result<(), String> {
    let (body, submit) = split_pane_submit(text);
    let Some(submit) = submit else {
        return if body.is_empty() {
            Ok(())
        } else {
            pane.lock()
                .map_err(|error| error.to_string())?
                .write_input(body.as_bytes())
                .map_err(|error| error.to_string())
        };
    };
    let pty = pane.lock().map_err(|error| error.to_string())?.shared_pty();
    let reservation = pty
        .reserve_input_transaction()
        .map_err(|error| error.to_string())?;
    if !body.is_empty() {
        // Issue #4909: a submit-bearing body is a prompt, so it is pasted
        // when the composer accepts pastes.
        reservation
            .write_input(&pane_input_body_bytes(
                body,
                reservation.bracketed_paste_enabled(),
            )?)
            .map_err(|error| error.to_string())?;
    }
    let submit = submit.to_string();
    thread::spawn(move || {
        thread::sleep(PANE_SUBMIT_SETTLE);
        if let Err(error) = reservation.write_input(submit.as_bytes()) {
            tracing::warn!(%error, "pane submit byte could not be written");
        }
    });
    Ok(())
}

/// Write one pane payload and wait for the physical submit byte to complete.
/// This is only the physical boundary for autonomous-answer delivery; the
/// provider's UserPromptSubmit hook remains the durable acknowledgment. Run it
/// through an owning runtime worker rather than the application loop.
pub fn write_pane_input_and_submit_blocking(
    pane: &Arc<Mutex<Pane>>,
    text: &str,
) -> Result<(), String> {
    let (body, submit) = split_pane_submit(text);
    let Some(submit) = submit else {
        return if body.is_empty() {
            Ok(())
        } else {
            pane.lock()
                .map_err(|error| error.to_string())?
                .write_input(body.as_bytes())
                .map_err(|error| error.to_string())
        };
    };
    let pty = pane.lock().map_err(|error| error.to_string())?.shared_pty();
    let reservation = pty
        .reserve_input_transaction()
        .map_err(|error| error.to_string())?;
    if !body.is_empty() {
        // Issue #4909: see `write_pane_input_then_submit`.
        reservation
            .write_input(&pane_input_body_bytes(
                body,
                reservation.bracketed_paste_enabled(),
            )?)
            .map_err(|error| error.to_string())?;
    }
    thread::sleep(PANE_SUBMIT_SETTLE);
    reservation
        .write_input(submit.to_string().as_bytes())
        .map_err(|error| error.to_string())
}

pub fn workspace_state_reply(
    client_id: &str,
    principal: &AgentSessionPrincipal,
    app_version: String,
    tabs: Vec<(PathBuf, ProjectTabView)>,
) -> OutboundEvent {
    let tabs = tabs
        .into_iter()
        .filter(|(root, _)| principal.authorizes_project_root(root))
        .map(|(_, tab)| tab)
        .collect::<Vec<_>>();
    let active_tab_id = tabs.first().map(|tab| tab.id.clone());
    OutboundEvent::reply(
        client_id,
        BackendEvent::WorkspaceState {
            workspace: crate::AppStateView {
                app_version,
                tabs,
                active_tab_id,
                recent_projects: Vec::new(),
            },
        },
    )
}

/// Immutable snapshot inputs borrow the one physical owner's parser handles.
pub struct PaneSnapshotInput {
    pub id: String,
    pub pane: Option<Arc<Mutex<Pane>>>,
    pub cached_snapshot: Option<(Vec<u8>, u64)>,
    pub launch_error_snapshot: Option<Vec<u8>>,
    pub launch_error_source: Option<gwt_agent::Session>,
}

pub fn frontend_sync_events(
    client_id: &str,
    workspace_state: OutboundEvent,
    mut snapshots: Vec<PaneSnapshotInput>,
) -> Vec<OutboundEvent> {
    let mut events = vec![workspace_state];
    let mut empty_window_ids = Vec::new();
    let mut busy_window_ids = Vec::new();
    let mut unavailable_window_ids = Vec::new();
    let mut failed_window_ids = Vec::new();
    snapshots.sort_by(|left, right| left.id.cmp(&right.id));
    for input in snapshots {
        let snapshot_lock_started = Instant::now();
        let launch_error_snapshot = input.launch_error_snapshot.filter(|_| {
            input.launch_error_source.as_ref().is_none_or(|expected| {
                gwt_agent::Session::load(&gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", expected.id)))
                    .is_ok_and(|current| {
                        let expected = super::events::launch_failure_session_fingerprint(expected);
                        let current = super::events::launch_failure_session_fingerprint(&current);
                        matches!((expected, current), (Ok(expected), Ok(current)) if expected == current)
                    })
            })
        });
        let id = input.id;
        let mut stream_seq = None;
        let has_runtime = input.pane.is_some() || input.cached_snapshot.is_some();
        let snapshot = match &input.pane {
            Some(pane) => match pane.try_lock() {
                Ok(pane) => {
                    let snapshot_started = Instant::now();
                    let snapshot = pane.snapshot_bytes();
                    stream_seq = Some(pane.output_seq());
                    let elapsed_ms =
                        u64::try_from(snapshot_started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    if elapsed_ms >= 500 {
                        tracing::warn!(target: "gwt.pane.sync", window_id = %id, stage = "pane_snapshot_serialize", elapsed_ms, outcome = "slow", "pane snapshot serialization exceeded the GUI-loop budget");
                    }
                    if snapshot.is_empty() {
                        launch_error_snapshot.clone()
                    } else {
                        Some(snapshot)
                    }
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    tracing::warn!(target: "gwt.pane.sync", window_id = %id, stage = "pane_snapshot_try_lock", elapsed_ms = u64::try_from(snapshot_lock_started.elapsed().as_millis()).unwrap_or(u64::MAX), outcome = "busy", "pane snapshot skipped a contended runtime");
                    busy_window_ids.push(id.clone());
                    None
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    tracing::error!(target: "gwt.pane.sync", window_id = %id, stage = "pane_snapshot_try_lock", elapsed_ms = u64::try_from(snapshot_lock_started.elapsed().as_millis()).unwrap_or(u64::MAX), outcome = "poisoned", "pane snapshot skipped a poisoned runtime");
                    failed_window_ids.push(id.clone());
                    None
                }
            },
            None => {
                if let Some((snapshot, seq)) = input.cached_snapshot {
                    stream_seq = Some(seq);
                    if snapshot.is_empty() {
                        launch_error_snapshot
                    } else {
                        Some(snapshot)
                    }
                } else {
                    launch_error_snapshot
                }
            }
        };
        if let Some(snapshot) = snapshot {
            events.push(
                OutboundEvent::reply(
                    client_id,
                    BackendEvent::TerminalSnapshot {
                        id,
                        data_base64: base64::engine::general_purpose::STANDARD.encode(snapshot),
                    },
                )
                .with_terminal_stream_seq(stream_seq),
            );
        } else if !busy_window_ids.contains(&id) && !failed_window_ids.contains(&id) {
            if has_runtime {
                empty_window_ids.push(id);
            } else {
                unavailable_window_ids.push(id);
            }
        }
    }
    events.push(OutboundEvent::reply(
        client_id,
        BackendEvent::PaneSyncComplete {
            empty_window_ids,
            busy_window_ids,
            unavailable_window_ids,
            failed_window_ids,
        },
    ));
    events
}

pub fn pane_snapshot_repair_events(
    client_id: &str,
    snapshots: Vec<PaneSnapshotInput>,
) -> Vec<OutboundEvent> {
    snapshots
        .into_iter()
        .filter_map(|input| {
            let (snapshot, seq) = if let Some(pane) = input.pane {
                pane.lock()
                    .map(|pane| (pane.snapshot_bytes(), pane.output_seq()))
                    .unwrap_or_default()
            } else {
                input.cached_snapshot?
            };
            (!snapshot.is_empty()).then(|| {
                OutboundEvent::reply(
                    client_id,
                    BackendEvent::TerminalSnapshot {
                        id: input.id,
                        data_base64: base64::engine::general_purpose::STANDARD.encode(snapshot),
                    },
                )
                .with_terminal_stream_seq(Some(seq))
            })
        })
        .collect()
}

use crate::{pm_registry, WindowPreset, WindowProcessStatus};
const PM_DELIVERY_MAX_BODY_BYTES: usize = 16 * 1024;

/// Reserved before the operation deadline for terminalizing the operation.
/// The receipt commit takes a file lock and an fsync under the same scoped
/// deadline, and the reply still has to reach the origin socket before the
/// server's acceptance deadline — spending the budget down to the last
/// millisecond would turn a clean `unverified` answer back into a timeout.
const PM_DELIVERY_TERMINAL_MARGIN: Duration = Duration::from_millis(500);

/// How often the durable receipt is re-read while waiting for the target's
/// acknowledgement.
const PM_DELIVERY_ACK_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// A physical write does not prove that the target started a turn. In the
/// original idle-pane failure the body stayed in the composer after submit
/// was dropped. Keep the operation identity; never stage its body again.
const PM_DELIVERY_UNCONFIRMED_REASON: &str =
    "PM pane delivery was not acknowledged; the prompt may still be unsent. Recheck the same \
     operation_id with the same target and text, never a new operation. Submit-only recovery \
     requires the original fully written body and unchanged target input";

const PM_DELIVERY_LOST_REASON: &str =
    "The original target session is unavailable for submit recovery. Turn start remains \
     unconfirmed; a late exact acknowledgement can still confirm this same operation";

/// Wait for the target Session's acknowledgement to reach the durable receipt,
/// and report the operation's latest durable status.
///
/// Verification is asynchronous and out-of-process: the acknowledgement is
/// written by the target's own `UserPromptSubmit` hook, which has to start,
/// run its steps, and commit — routinely far longer than the submit-retry
/// budget. Issue #3608 recorded the consequence live (`prepared 02:40:38 →
/// ambiguous 02:40:39 → verified 02:40:40`): a delivery that had already been
/// submitted was terminalized as unverified one second before its
/// acknowledgement landed, with most of the operation's budget still unspent.
/// So the wait runs to the deadline and returns early only once the operation
/// is durably terminal.
fn await_pm_delivery_status(
    receipt_path: &Path,
    operation_id: &str,
    poll_deadline: Instant,
) -> Result<pm_registry::PmDeliveryReceiptStatus, String> {
    loop {
        let status = pm_registry::pm_delivery_receipt_for_operation(receipt_path, operation_id)
            .map_err(|error| format!("PM delivery receipt is unavailable: {error}"))?
            .map(|receipt| receipt.status)
            .unwrap_or(pm_registry::PmDeliveryReceiptStatus::Prepared);
        if matches!(
            status,
            pm_registry::PmDeliveryReceiptStatus::Verified
                | pm_registry::PmDeliveryReceiptStatus::Refused
                | pm_registry::PmDeliveryReceiptStatus::Lost
        ) || Instant::now() >= poll_deadline
        {
            return Ok(status);
        }
        thread::sleep(
            PM_DELIVERY_ACK_POLL_INTERVAL
                .min(poll_deadline.saturating_duration_since(Instant::now())),
        );
    }
}

#[derive(Clone)]
pub struct PaneProjection {
    pub project_root: PathBuf,
    pub tab_id: String,
    pub window: crate::persistence::PersistedWindowState,
    pub active: Option<super::launch_payload::ActiveAgentSession>,
}

pub fn live_pm_window_id(panes: &[PaneProjection], session_id: &str) -> Option<String> {
    panes
        .iter()
        .find(|pane| {
            pane.active
                .as_ref()
                .is_some_and(|active| active.session_id == session_id)
                && !matches!(
                    pane.window.status,
                    crate::WindowProcessStatus::Stopped | crate::WindowProcessStatus::Error
                )
        })
        .map(|pane| pane.window.id.clone())
}

#[allow(clippy::too_many_arguments)]
pub fn authenticated_pm_pane_send_input_events(
    issuer: &AgentCapabilityIssuer,
    client_id: String,
    grant: AgentCapabilityGrant,
    operation_id: &str,
    window_id: &str,
    text: &str,
    responder: Option<AgentPmSendResponder>,
    panes: Vec<PaneProjection>,
    writers: &PtyWriterRegistry,
    spawn: impl FnOnce(Box<dyn FnOnce() + Send + 'static>) -> Result<(), String>,
    emit: Arc<dyn Fn(OutboundEvent) + Send + Sync>,
) -> Vec<OutboundEvent> {
    let finish = |status: &str, reason: Option<String>| {
        let event = BackendEvent::PmMessageSendResult {
            operation_id: operation_id.to_string(),
            status: status.to_string(),
            window_id: Some(window_id.to_string()),
            reason,
        };
        if let Some(responder) = responder.as_ref() {
            let _ = responder.send(event);
            Vec::new()
        } else {
            vec![OutboundEvent::reply(client_id.clone(), event)]
        }
    };
    if uuid::Uuid::parse_str(operation_id)
        .ok()
        .is_none_or(|parsed| parsed.hyphenated().to_string() != operation_id)
    {
        return finish(
            "failed",
            Some("pm.message.send requires a canonical operation UUID".to_string()),
        );
    }
    if !issuer.grant_is_current(&grant)
        || responder
            .as_ref()
            .is_some_and(|responder| !responder.mutation_is_current())
    {
        return finish(
            "failed",
            Some("pm.message.send capability is stale".to_string()),
        );
    }

    let project_root = grant.principal().canonical_project_root().to_path_buf();
    let principal_session_id = grant.principal().session_id().to_string();
    let prefs_path = pm_registry::pm_prefs_path_for_repo_path(&project_root);
    let registration = match pm_registry::load_pm_prefs(&prefs_path) {
        Ok(prefs) => prefs.registration,
        Err(error) => {
            return finish(
                "failed",
                Some(format!(
                    "pm.message.send registration is unreadable: {error}"
                )),
            )
        }
    };
    let Some(registration) =
        registration.filter(|registration| registration.session_id == principal_session_id)
    else {
        return finish(
            "failed",
            Some("pm.message.send caller is not the live registered PM".to_string()),
        );
    };
    let Some(principal_window_id) = live_pm_window_id(&panes, &principal_session_id) else {
        return finish(
            "failed",
            Some("pm.message.send caller is not the live registered PM".to_string()),
        );
    };
    let principal_pty = match writers
        .read()
        .map_err(|_| "pm.message.send PTY registry is unavailable".to_string())
        .and_then(|writers| {
            writers
                .get(&principal_window_id)
                .map(|entry| Arc::clone(&entry.handle))
                .ok_or_else(|| "pm.message.send caller has no live PTY".to_string())
        }) {
        Ok(pty) => pty,
        Err(error) => return finish("failed", Some(error)),
    };

    let Some(body) = text.strip_suffix('\r') else {
        return finish(
            "failed",
            Some("pm.message.send requires one line terminated by a carriage return".to_string()),
        );
    };
    if body.len() > PM_DELIVERY_MAX_BODY_BYTES {
        return finish(
            "failed",
            Some(format!(
                "pm.message.send body exceeds the {PM_DELIVERY_MAX_BODY_BYTES}-byte limit"
            )),
        );
    }
    let protected_prompt = match pm_registry::protected_pm_delivery_prompt(operation_id, body) {
        Ok(prompt) => prompt,
        Err(error) => return finish("failed", Some(error.to_string())),
    };
    let target = panes.iter().find_map(|pane| {
        let canonical_root = dunce::canonicalize(&pane.project_root)
            .map(|path| gwt_core::paths::normalize_windows_child_process_path(&path))
            .ok()?;
        if canonical_root != project_root || pane.window.id != window_id {
            return None;
        }
        Some((
            pane.tab_id.clone(),
            pane.window.preset,
            pane.window.status,
            pane.window.session_id.clone(),
            pane.window.is_pm
                || pm_registry::pane_is_pm(
                    &project_root,
                    pane.active
                        .as_ref()
                        .map(|active| active.worktree_path.as_path()),
                    pane.window.session_id.as_deref(),
                ),
        ))
    });
    let target_session_id = target
        .as_ref()
        .and_then(|(_, _, _, session_id, _)| session_id.clone());
    let self_delivery_reason = (target_session_id.as_deref() == Some(principal_session_id.as_str())
            || target.as_ref().is_some_and(|(_, _, _, _, is_pm)| *is_pm))
            .then(|| format!("pm.message.send refused self-delivery to PM pane {window_id}; choose an implementation_agent from pane.list"));
    let target_is_live_agent = target.as_ref().is_some_and(
        |(target_tab_id, target_preset, target_status, target_session_id, _)| {
            matches!(
                target_status,
                WindowProcessStatus::Running
                    | WindowProcessStatus::Idle
                    | WindowProcessStatus::Waiting
            ) && target_session_id
                .as_deref()
                .is_some_and(|target_session_id| {
                    panes
                        .iter()
                        .find(|pane| pane.window.id == window_id)
                        .and_then(|pane| pane.active.as_ref())
                        .is_some_and(|session| {
                            let exact_prompt_ack =
                                matches!(target_preset, WindowPreset::Claude | WindowPreset::Codex)
                                    || (*target_preset == WindowPreset::Agent
                                        && matches!(session.agent_id.as_str(), "claude" | "codex"));
                            exact_prompt_ack
                                && session.window_id == window_id
                                && session.tab_id == *target_tab_id
                                && session.session_id == target_session_id
                        })
                })
        },
    );
    let expected_entry = if target_is_live_agent {
        writers
            .read()
            .ok()
            .and_then(|writers| writers.get(window_id).cloned())
    } else {
        None
    };
    let expected_pty = expected_entry
        .as_ref()
        .map(|entry| Arc::clone(&entry.handle));

    let operation_id = operation_id.to_string();
    let window_id = window_id.to_string();
    let body_sha256 = pm_registry::pm_delivery_prompt_sha256(body);
    let receipt_path = pm_registry::pm_delivery_receipts_path_for_repo_path(&project_root);
    let writers = Arc::clone(writers);
    let issuer = issuer.clone();
    let worker_responder = responder.clone();
    let worker_client_id = client_id.clone();
    let worker_operation_id = operation_id.clone();
    let worker_window_id = window_id.clone();
    let spawn = spawn(Box::new(move || {
        let send_terminal = |status: &str, reason: Option<String>| {
            let event = BackendEvent::PmMessageSendResult {
                operation_id: worker_operation_id.clone(),
                status: status.to_string(),
                window_id: Some(worker_window_id.clone()),
                reason,
            };
            if let Some(responder) = worker_responder.as_ref() {
                let _ = responder.send(event);
            } else {
                emit(OutboundEvent::reply(worker_client_id.clone(), event));
            }
        };
        let deadline = worker_responder
            .as_ref()
            .map(AgentPmSendResponder::deadline)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(2));
        let _operation_deadline =
            gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline);
        // The whole budget is available for the acknowledgement; only the
        // terminal receipt commit and the reply are held back.
        let ack_deadline = deadline
            .checked_sub(PM_DELIVERY_TERMINAL_MARGIN)
            .unwrap_or(deadline);
        let mut body_attempted = false;
        let mut receipt_prepared = false;
        let mut receipt_terminalized = false;
        let mut durable_target_session_id = target_session_id.clone();
        let prepare =
            pm_registry::pm_delivery_receipt_for_operation(&receipt_path, &worker_operation_id)
                .map_err(|error| format!("PM delivery receipt replay failed: {error}"))
                .and_then(|existing| {
                    if let Some(existing) = existing {
                        if existing.principal_session_id != principal_session_id
                            || existing.target_window_id != worker_window_id
                            || existing.body_sha256 != body_sha256
                        {
                            return Err(
                                "PM delivery operation identity conflicts with its durable receipt"
                                    .to_string(),
                            );
                        }
                        durable_target_session_id = Some(existing.target_session_id);
                        return Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                            existing.status,
                        ));
                    }
                    let target_session_id =
                        durable_target_session_id.as_ref().ok_or_else(|| {
                            "pm.message.send refused: target is not an authorized live agent pane"
                                .to_string()
                        })?;
                    if self_delivery_reason.is_none()
                        && (!target_is_live_agent || expected_pty.is_none())
                    {
                        return Err(
                            "pm.message.send refused: target is not an authorized live agent pane"
                                .to_string(),
                        );
                    }
                    let prepared = pm_registry::PmDeliveryReceipt {
                        operation_id: worker_operation_id.clone(),
                        recorded_at: chrono::Utc::now()
                            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                        status: pm_registry::PmDeliveryReceiptStatus::Prepared,
                        principal_session_id: principal_session_id.clone(),
                        target_window_id: worker_window_id.clone(),
                        target_session_id: target_session_id.clone(),
                        body_sha256: body_sha256.clone(),
                        reason: None,
                    };
                    pm_registry::prepare_pm_delivery_receipt(&receipt_path, &prepared).map_err(
                        |error| format!("PM delivery receipt Prepared commit failed: {error}"),
                    )
                });
        let mut replay_attempted = false;
        let mut readback_only = false;
        let mut target_lost = false;
        // A receipt alone never proves that its body is still in this
        // composer. Keep a generation-local guard only after a full body
        // write, consumed by one bounded submit-only recovery.
        let mut drive_pane = |payload: &str, replay: Option<PmDeliveryReplayGuard>| {
            let delivery_target_session_id = durable_target_session_id
                .clone()
                .expect("Prepared PM delivery has a target Session");
            let expected_pty = expected_pty.as_ref().ok_or_else(|| {
                "pm.message.send refused: target is not an authorized live agent pane".to_string()
            })?;
            let ordinary_revision = replay
                .as_ref()
                .map(|guard| guard.ordinary_revision)
                .or_else(|| expected_pty.ordinary_input_revision())
                .ok_or_else(|| "PM input revision is unavailable; readback-only".to_string())?;
            if expected_pty.ordinary_input_revision() != Some(ordinary_revision)
                || expected_pty.has_unsent_user_input()
            {
                return Err("PM input changed; readback-only recovery".to_string());
            }
            let reservation = Arc::clone(expected_pty)
                .reserve_input_transaction()
                .map_err(|error| format!("PM pane input transaction unavailable: {error}"))?;
            if replay.as_ref().is_some_and(|guard| {
                reservation.previous_transaction_revision() != guard.transaction_revision
            }) {
                return Err(
                    "another protected input intervened; readback-only recovery".to_string()
                );
            }
            let mut body_written = false;
            // Issue #4909: the mode is read on the reservation the
            // body is written through, so the paste wrapping and
            // the write see the same composer state.
            let bracketed_paste = reservation.bracketed_paste_enabled();
            let outcome = drive_verified_pane_submit(
                payload,
                bracketed_paste,
                2,
                |bytes| {
                    let written = reservation
                        .write_input_authorized(bytes, |commit| {
                            if Instant::now() >= deadline
                                || worker_responder
                                    .as_ref()
                                    .is_some_and(|responder| !responder.mutation_is_current())
                            {
                                return Err(gwt_terminal::TerminalError::PtyIoError {
                                    details: "PM delivery deadline expired before input mutation"
                                        .to_string(),
                                });
                            }
                            let current_registration = pm_registry::load_pm_prefs(&prefs_path)
                                .map_err(|error| gwt_terminal::TerminalError::PtyIoError {
                                    details: format!(
                                        "PM registration revalidation failed: {error}"
                                    ),
                                })?
                                .registration;
                            if current_registration.as_ref() != Some(&registration) {
                                return Err(gwt_terminal::TerminalError::PtyIoError {
                                    details:
                                        "registered PM authority changed before input mutation"
                                            .to_string(),
                                });
                            }
                            {
                                let current = writers.read().map_err(|_| {
                                    gwt_terminal::TerminalError::PtyIoError {
                                        details: "PM delivery PTY registry is unavailable"
                                            .to_string(),
                                    }
                                })?;
                                if !current.get(&worker_window_id).is_some_and(|entry| {
                                    expected_entry.as_ref().is_some_and(|expected| {
                                        Arc::ptr_eq(entry, expected)
                                            && entry
                                                .current
                                                .load(std::sync::atomic::Ordering::Acquire)
                                    })
                                }) || !current.get(&principal_window_id).is_some_and(|entry| {
                                    Arc::ptr_eq(&entry.handle, &principal_pty)
                                        && entry.current.load(std::sync::atomic::Ordering::Acquire)
                                }) {
                                    return Err(gwt_terminal::TerminalError::PtyIoError {
                                        details: "PM caller or target runtime generation changed"
                                            .to_string(),
                                    });
                                }
                            }
                            if Instant::now() >= deadline
                                || worker_responder
                                    .as_ref()
                                    .is_some_and(|responder| !responder.mutation_is_current())
                            {
                                return Err(gwt_terminal::TerminalError::PtyIoError {
                                    details: "PM delivery deadline expired before input commit"
                                        .to_string(),
                                });
                            }
                            if !issuer.commit_mutation_if_current(&grant, || {
                                worker_responder
                                    .as_ref()
                                    .is_none_or(AgentPmSendResponder::try_commit_mutation)
                            }) {
                                return Err(gwt_terminal::TerminalError::PtyIoError {
                                    details:
                                        "PM capability or origin changed before input mutation"
                                            .to_string(),
                                });
                            }
                            let mut mark_attempted = || {
                                if bytes != b"\r" {
                                    body_attempted = true;
                                }
                            };
                            reservation.with_unchanged_ordinary_input(ordinary_revision, || {
                                commit(&mut mark_attempted)
                            })
                        })
                        .map_err(|error| error.to_string());
                    if written.is_ok() && bytes != b"\r" {
                        body_written = true;
                    }
                    written
                },
                |settle| {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    thread::sleep(settle.min(remaining));
                },
                || {
                    pm_registry::pm_delivery_receipt_for_operation(
                        &receipt_path,
                        &worker_operation_id,
                    )
                    .map_err(|error| format!("PM delivery acknowledgement is unavailable: {error}"))
                    .map(|receipt| {
                        receipt.is_some_and(|receipt| {
                            receipt.target_session_id == delivery_target_session_id
                                && receipt.body_sha256 == body_sha256
                                && receipt.status == pm_registry::PmDeliveryReceiptStatus::Verified
                        })
                    })
                },
            );
            if body_written && replay.is_none() {
                let entry = expected_entry.as_ref().expect("live PM target entry");
                *entry
                    .pm_delivery_replay
                    .lock()
                    .map_err(|_| "PM recovery guard is unavailable".to_string())? =
                    Some(PmDeliveryReplayGuard {
                        operation_id: worker_operation_id.clone(),
                        target_session_id: delivery_target_session_id,
                        ordinary_revision,
                        transaction_revision: reservation.transaction_revision(),
                    });
            }
            outcome
        };
        let result = match prepare {
            Ok(pm_registry::PmDeliveryPrepareOutcome::Prepared) => {
                if let Some(reason) = self_delivery_reason.as_deref() {
                    match pm_registry::finish_pm_delivery_receipt(
                        &receipt_path,
                        &worker_operation_id,
                        durable_target_session_id
                            .as_deref()
                            .expect("Prepared target Session"),
                        &body_sha256,
                        pm_registry::PmDeliveryReceiptStatus::Refused,
                        Some(reason),
                    ) {
                        Ok(pm_registry::PmDeliveryReceiptStatus::Refused) => {
                            send_terminal("refused", Some(reason.to_string()))
                        }
                        Ok(_) => send_terminal(
                            "failed",
                            Some("PM self-delivery receipt changed before refusal".to_string()),
                        ),
                        Err(error) => send_terminal(
                            "failed",
                            Some(format!(
                                "PM self-delivery refusal receipt commit failed: {error}"
                            )),
                        ),
                    }
                    return;
                }
                receipt_prepared = true;
                drive_pane(&protected_prompt, None)
            }
            Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                pm_registry::PmDeliveryReceiptStatus::Verified,
            )) => {
                receipt_terminalized = true;
                Ok(VerifiedPaneSubmitOutcome::Verified { submit_attempts: 0 })
            }
            Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                pm_registry::PmDeliveryReceiptStatus::Prepared,
            )) => {
                let target_session_id = durable_target_session_id
                    .as_ref()
                    .expect("existing Prepared has a target Session");
                match await_pm_delivery_status(&receipt_path, &worker_operation_id, ack_deadline) {
                    Err(error) => Err(error),
                    Ok(pm_registry::PmDeliveryReceiptStatus::Verified) => {
                        receipt_terminalized = true;
                        Ok(VerifiedPaneSubmitOutcome::Verified { submit_attempts: 0 })
                    }
                    Ok(
                        pm_registry::PmDeliveryReceiptStatus::Ambiguous
                        | pm_registry::PmDeliveryReceiptStatus::Unconfirmed,
                    ) => {
                        receipt_terminalized = true;
                        Ok(VerifiedPaneSubmitOutcome::Unverified { submit_attempts: 0 })
                    }
                    Ok(pm_registry::PmDeliveryReceiptStatus::Lost) => {
                        receipt_terminalized = true;
                        target_lost = true;
                        Ok(VerifiedPaneSubmitOutcome::Unverified { submit_attempts: 0 })
                    }
                    Ok(pm_registry::PmDeliveryReceiptStatus::Refused) => {
                        let reason = pm_registry::pm_delivery_receipt_for_operation(
                            &receipt_path,
                            &worker_operation_id,
                        )
                        .ok()
                        .flatten()
                        .and_then(|receipt| receipt.reason)
                        .unwrap_or_else(|| {
                            format!(
                                "PM delivery operation was already refused for {worker_window_id}"
                            )
                        });
                        send_terminal("refused", Some(reason));
                        return;
                    }
                    Ok(pm_registry::PmDeliveryReceiptStatus::Prepared) => {
                        match pm_registry::finish_pm_delivery_receipt(
                            &receipt_path,
                            &worker_operation_id,
                            target_session_id,
                            &body_sha256,
                            pm_registry::PmDeliveryReceiptStatus::Unconfirmed,
                            Some("response_lost_after_prepare"),
                        )
                        .map_err(|error| {
                            format!("PM delivery replay could not terminalize Prepared: {error}")
                        }) {
                            Ok(pm_registry::PmDeliveryReceiptStatus::Verified) => {
                                receipt_terminalized = true;
                                Ok(VerifiedPaneSubmitOutcome::Verified { submit_attempts: 0 })
                            }
                            Ok(_) => {
                                receipt_terminalized = true;
                                Ok(VerifiedPaneSubmitOutcome::Unverified { submit_attempts: 0 })
                            }
                            Err(error) => Err(error),
                        }
                    }
                }
            }
            Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                pm_registry::PmDeliveryReceiptStatus::Ambiguous
                | pm_registry::PmDeliveryReceiptStatus::Unconfirmed,
            )) => {
                receipt_terminalized = true;
                let same_target = self_delivery_reason.is_none()
                    && target_is_live_agent
                    && expected_entry.is_some()
                    && target_session_id == durable_target_session_id;
                if !same_target {
                    match pm_registry::finish_pm_delivery_receipt(
                        &receipt_path,
                        &worker_operation_id,
                        durable_target_session_id
                            .as_deref()
                            .expect("durable target"),
                        &body_sha256,
                        pm_registry::PmDeliveryReceiptStatus::Lost,
                        Some("target_unavailable"),
                    ) {
                        Ok(pm_registry::PmDeliveryReceiptStatus::Verified) => {
                            Ok(VerifiedPaneSubmitOutcome::Verified { submit_attempts: 0 })
                        }
                        Ok(_) => {
                            target_lost = true;
                            Ok(VerifiedPaneSubmitOutcome::Unverified { submit_attempts: 0 })
                        }
                        Err(error) => Err(format!("PM target-loss receipt failed: {error}")),
                    }
                } else {
                    let guard = expected_entry.as_ref().and_then(|entry| {
                        let mut guard = entry.pm_delivery_replay.lock().ok()?;
                        if guard.as_ref().is_some_and(|guard| {
                            guard.operation_id == worker_operation_id
                                && Some(&guard.target_session_id)
                                    == durable_target_session_id.as_ref()
                        }) {
                            guard.take()
                        } else {
                            None
                        }
                    });
                    if let Some(guard) = guard {
                        replay_attempted = true;
                        drive_pane("\r", Some(guard))
                    } else {
                        readback_only = true;
                        Ok(VerifiedPaneSubmitOutcome::Unverified { submit_attempts: 0 })
                    }
                }
            }
            Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                pm_registry::PmDeliveryReceiptStatus::Lost,
            )) => {
                receipt_terminalized = true;
                target_lost = true;
                Ok(VerifiedPaneSubmitOutcome::Unverified { submit_attempts: 0 })
            }
            Ok(pm_registry::PmDeliveryPrepareOutcome::Existing(
                pm_registry::PmDeliveryReceiptStatus::Refused,
            )) => {
                let reason = pm_registry::pm_delivery_receipt_for_operation(
                    &receipt_path,
                    &worker_operation_id,
                )
                .ok()
                .flatten()
                .and_then(|receipt| receipt.reason)
                .unwrap_or_else(|| {
                    format!("PM delivery operation was already refused for {worker_window_id}")
                });
                send_terminal("refused", Some(reason));
                return;
            }
            Err(error) => Err(error),
        };

        match result {
            Ok(VerifiedPaneSubmitOutcome::Verified { .. }) => send_terminal("delivered", None),
            Ok(VerifiedPaneSubmitOutcome::Unverified { .. }) if target_lost => {
                if pm_registry::pm_delivery_receipt_for_operation(
                    &receipt_path,
                    &worker_operation_id,
                )
                .ok()
                .flatten()
                .is_some_and(|receipt| {
                    receipt.status == pm_registry::PmDeliveryReceiptStatus::Verified
                }) {
                    send_terminal("delivered", None);
                } else {
                    send_terminal("lost", Some(PM_DELIVERY_LOST_REASON.to_string()));
                }
            }
            Ok(VerifiedPaneSubmitOutcome::Unverified { .. }) => {
                let target_session_id = durable_target_session_id
                    .as_deref()
                    .expect("Prepared PM delivery has a target Session");
                // Issue #3608: exhausting the submit retries is not the end
                // of the operation. The acknowledgement crosses a process
                // boundary and regularly lands after them, so spend the
                // rest of the budget waiting for it before terminalizing.
                let _ = await_pm_delivery_status(&receipt_path, &worker_operation_id, ack_deadline);
                match pm_registry::finish_pm_delivery_receipt(
                    &receipt_path,
                    &worker_operation_id,
                    target_session_id,
                    &body_sha256,
                    pm_registry::PmDeliveryReceiptStatus::Unconfirmed,
                    Some("submit_unconfirmed"),
                ) {
                    Ok(pm_registry::PmDeliveryReceiptStatus::Verified) => {
                        send_terminal("delivered", None)
                    }
                    Ok(pm_registry::PmDeliveryReceiptStatus::Lost) => {
                        send_terminal("lost", Some(PM_DELIVERY_LOST_REASON.to_string()))
                    }
                    status => {
                        let recovery = if readback_only || replay_attempted {
                            " Further calls are readback-only; no eligible submit recovery remains."
                        } else {
                            ""
                        };
                        let persistence = status
                            .err()
                            .map(|error| format!(" Receipt commit failed: {error}"))
                            .unwrap_or_default();
                        send_terminal(
                            "unconfirmed",
                            Some(format!(
                                "{PM_DELIVERY_UNCONFIRMED_REASON}{recovery}{persistence}"
                            )),
                        );
                    }
                }
            }
            Err(mut error) => {
                let mut verified_during_finalization = false;
                if receipt_prepared && !receipt_terminalized {
                    let target_session_id = durable_target_session_id
                        .as_deref()
                        .expect("Prepared PM delivery has a target Session");
                    let status = if body_attempted {
                        pm_registry::PmDeliveryReceiptStatus::Unconfirmed
                    } else {
                        pm_registry::PmDeliveryReceiptStatus::Refused
                    };
                    match pm_registry::finish_pm_delivery_receipt(
                        &receipt_path,
                        &worker_operation_id,
                        target_session_id,
                        &body_sha256,
                        status,
                        Some(if body_attempted {
                            "body_staged_or_unknown"
                        } else {
                            "mutation_refused"
                        }),
                    ) {
                        Ok(pm_registry::PmDeliveryReceiptStatus::Verified) => {
                            verified_during_finalization = true;
                        }
                        Ok(_) => {}
                        Err(receipt_error) => {
                            error.push_str(&format!(
                                "; PM delivery receipt terminal commit failed: {receipt_error}"
                            ));
                        }
                    }
                }
                if verified_during_finalization {
                    send_terminal("delivered", None);
                    return;
                }
                if body_attempted || replay_attempted || receipt_terminalized {
                    send_terminal("unconfirmed", Some(format!(
                            "{PM_DELIVERY_UNCONFIRMED_REASON}; readback-only until safe recovery is available: {error}")));
                } else {
                    send_terminal("failed", Some(error));
                }
            }
        }
    }));
    if let Err(error) = spawn {
        return finish(
            "failed",
            Some(format!("pm.message.send worker unavailable: {error}")),
        );
    }
    Vec::new()
}

pub fn principal_session_window(
    panes: &[PaneProjection],
    principal: &AgentSessionPrincipal,
) -> Result<String, String> {
    let mut matches = panes.iter().filter(|pane| {
        principal.authorizes_project_root(&pane.project_root)
            && pane.window.session_id.as_deref() == Some(principal.session_id())
    });
    let pane = matches
        .next()
        .ok_or_else(|| "authenticated session is not bound to this project".to_string())?;
    if matches.next().is_some() {
        return Err("authenticated session has multiple panes in this project".into());
    }
    Ok(pane.window.id.clone())
}

pub struct PreparedPaneName {
    window_id: String,
    name: String,
    _allocated: Option<crate::agent_names::GlobalAgentNameReservation>,
    _existing: Option<gwt_core::operation_deadline::NamedFileLock>,
}
impl PreparedPaneName {
    pub fn window_id(&self) -> &str {
        &self.window_id
    }
}

/// Acquire before the producing ECR/Session dispatch lease; no writer runs here.
pub fn prepare_pane_name(
    panes: &[PaneProjection],
    principal: &AgentSessionPrincipal,
    sessions_dir: &Path,
) -> Result<PreparedPaneName, String> {
    let id = principal_session_window(panes, principal)?;
    prepare_pane_name_for_window(panes, id, sessions_dir, false)
}

pub fn prepare_legacy_pane_name(
    panes: &[PaneProjection],
    window_id: String,
    sessions_dir: &Path,
) -> Result<PreparedPaneName, String> {
    prepare_pane_name_for_window(panes, window_id, sessions_dir, true)
}

fn prepare_pane_name_for_window(
    panes: &[PaneProjection],
    id: String,
    sessions_dir: &Path,
    allow_fixed_pm: bool,
) -> Result<PreparedPaneName, String> {
    let pane = panes
        .iter()
        .find(|pane| pane.window.id == id)
        .ok_or("Agent window is missing")?;
    let session = pane
        .window
        .session_id
        .as_ref()
        .and_then(|id| gwt_agent::Session::load(&sessions_dir.join(format!("{id}.toml"))).ok());
    if pane.window.is_pm
        || crate::pm_registry::session_is_registered_pm(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&pane.project_root),
            pane.window.session_id.as_deref().unwrap_or_default(),
        )
        || session.as_ref().is_some_and(|session| {
            crate::pm_registry::is_canonical_pm_worktree(&session.worktree_path)
        })
    {
        if !allow_fixed_pm {
            return Err("PM windows keep their fixed identity".into());
        }
        let name = pane.window.agent_name.clone().or_else(|| {
            session
                .as_ref()
                .and_then(|session| session.agent_name.clone())
        });
        if let Some(name) = name {
            let lock = crate::agent_names::lock_global_agent_names()?;
            crate::agent_names::remember(
                &gwt_core::paths::gwt_project_dir_for_repo_path(&pane.project_root)
                    .join("agent-names.jsonl"),
                &HashSet::from([name.clone()]),
            )?;
            return Ok(PreparedPaneName {
                window_id: id,
                name,
                _allocated: None,
                _existing: Some(lock),
            });
        }
    }
    if let Some(name) = &pane.window.agent_name {
        let lock = crate::agent_names::lock_global_agent_names()?;
        return Ok(PreparedPaneName {
            window_id: id,
            name: name.clone(),
            _allocated: None,
            _existing: Some(lock),
        });
    }
    let retained = panes
        .iter()
        .filter(|pane| pane.window.preset.is_agent_terminal())
        .filter_map(|pane| {
            pane.window
                .agent_name
                .as_ref()
                .map(|name| crate::agent_names::RetainedAgentName {
                    project_root: pane.project_root.clone(),
                    window_id: pane.window.id.clone(),
                    session_id: pane.window.session_id.clone(),
                    name: name.clone(),
                })
        })
        .collect::<Vec<_>>();
    let reserved = crate::agent_names::reserve_global_agent_name(
        &pane.project_root,
        sessions_dir,
        Some(&id),
        pane.window.session_id.as_deref(),
        session.and_then(|session| session.agent_name),
        &retained,
    )?;
    Ok(PreparedPaneName {
        window_id: id,
        name: reserved.name().to_string(),
        _allocated: Some(reserved),
        _existing: None,
    })
}

pub struct AssignedPaneName {
    pub window_id: String,
    pub name: String,
    pub already_named: bool,
}
/// The caller holds the prepared global reservation and the accepted grant/lease.
pub fn assign_prepared_pane_name(
    panes: &[PaneProjection],
    sessions_dir: &Path,
    prepared: &PreparedPaneName,
) -> Result<AssignedPaneName, String> {
    assign_prepared_name_inner(panes, sessions_dir, prepared, false)
}
pub fn assign_legacy_pane_name(
    panes: &[PaneProjection],
    sessions_dir: &Path,
    prepared: &PreparedPaneName,
) -> Result<AssignedPaneName, String> {
    assign_prepared_name_inner(panes, sessions_dir, prepared, true)
}

/// The physical owner also names retained rows whose provider never requests
/// pane.name. Keep the same global reservation and Session CAS as that request.
pub(crate) fn backfill_owner_pane_names(
    root: &Path,
    sessions_dir: &Path,
    registry: &OwnerWindowRegistry,
) -> Result<crate::persistence::PersistedWindowRegistry, String> {
    let project_key = gwt_core::paths::project_scope_hash(root).to_string();
    let mut panes: Vec<_> = registry
        .lock()
        .map_err(|error| error.to_string())?
        .persisted()
        .windows
        .iter()
        .cloned()
        .map(|mut window| {
            qualify_window_projection(&mut window, &project_key);
            PaneProjection {
                project_root: root.to_path_buf(),
                tab_id: project_key.clone(),
                window,
                active: None,
            }
        })
        .collect();
    for index in 0..panes.len() {
        if !panes[index].window.preset.is_agent_terminal() {
            continue;
        }
        let result = (|| {
            if let Some(session_id) = &panes[index].window.session_id {
                let session =
                    gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml")))
                        .map_err(|error| error.to_string())?;
                if !super::continuation::worktree_matches_project_state(
                    session
                        .project_state_root
                        .as_deref()
                        .unwrap_or(&session.worktree_path),
                    root,
                ) {
                    return Err("Agent Session is outside the fixed project".to_string());
                }
            }
            let prepared =
                prepare_legacy_pane_name(&panes, panes[index].window.id.clone(), sessions_dir)?;
            let assigned = assign_legacy_pane_name(&panes, sessions_dir, &prepared)?;
            let raw = raw_owner_window_id(root, &assigned.window_id)?;
            let mut registry = registry.lock().map_err(|error| error.to_string())?;
            let row = registry.window(raw).ok_or("Agent window is missing")?;
            if row.session_id != panes[index].window.session_id
                || row.agent_name != panes[index].window.agent_name
            {
                return Err("Agent window changed before name publication".into());
            }
            if row.agent_name.as_ref() != Some(&assigned.name) {
                registry.set_agent_name(raw, Some(assigned.name.clone()));
                crate::persistence::save_workspace_state_durable(
                    &crate::persistence::workspace_state_path(root),
                    &registry.persistable_state(),
                )
                .map_err(|error| error.to_string())?;
                panes[index].window.agent_name = Some(assigned.name);
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            tracing::warn!(%error, window_id = %panes[index].window.id,
                "owner retained Agent naming for a later layout update or pane.name request");
        }
    }
    Ok(registry
        .lock()
        .map_err(|error| error.to_string())?
        .persisted()
        .clone())
}

fn assign_prepared_name_inner(
    panes: &[PaneProjection],
    sessions_dir: &Path,
    prepared: &PreparedPaneName,
    allow_fixed_pm: bool,
) -> Result<AssignedPaneName, String> {
    let pane = panes
        .iter()
        .find(|pane| pane.window.id == prepared.window_id)
        .ok_or("Agent window is missing")?;
    if !pane.window.preset.is_agent_terminal() {
        return Err("Agent window is missing".into());
    }
    let mut session = pane
        .window
        .session_id
        .as_ref()
        .and_then(|id| gwt_agent::Session::load(&sessions_dir.join(format!("{id}.toml"))).ok());
    let is_pm = pane.window.is_pm
        || crate::pm_registry::session_is_registered_pm(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&pane.project_root),
            pane.window.session_id.as_deref().unwrap_or_default(),
        )
        || session.as_ref().is_some_and(|session| {
            crate::pm_registry::is_canonical_pm_worktree(&session.worktree_path)
        });
    if is_pm && !allow_fixed_pm {
        return Err("PM windows keep their fixed identity".into());
    }
    if session
        .as_ref()
        .and_then(|session| session.agent_name.as_deref())
        .is_some_and(|name| name != prepared.name)
    {
        return Err("Session changed during Agent naming; retry on the next tick".into());
    }
    if let Some(session) = &mut session {
        if !is_pm
            && !matches!(
                session.status,
                gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
            )
            && !session.review_dispatch
            && !crate::session_inventory::observe_session(session, sessions_dir)
                .sessions
                .is_empty()
        {
            crate::agent_names::adopt_project_member_session(
                &pane.project_root,
                sessions_dir,
                session,
            )?;
        }
        if session.agent_name.is_none() {
            let previous = session.clone();
            session.agent_name = Some(prepared.name.clone());
            let saved = if gwt_agent::current_thread_holds_session_lease() {
                session.save_if_unchanged_under_lease(sessions_dir, &previous)
            } else {
                session.save_if_unchanged(sessions_dir, &previous)
            }
            .map_err(|error| error.to_string())?;
            if !saved {
                return Err("Session changed during Agent naming; retry on the next tick".into());
            }
        }
    }
    Ok(AssignedPaneName {
        window_id: prepared.window_id.clone(),
        name: prepared.name.clone(),
        already_named: pane.window.agent_name.is_some(),
    })
}

pub fn agent_close_board_check(
    root: &Path,
    principal: &AgentSessionPrincipal,
    target_session: Option<&str>,
    target_owner: Option<u64>,
    window_id: &str,
) -> Option<String> {
    match crate::board_read_receipts::recent_owner_unread(
        root,
        principal.session_id(),
        target_session,
        target_owner,
        chrono::Utc::now(),
    ) {
        Ok(entries) if !entries.is_empty() => Some(
            crate::board_read_receipts::read_required_message("pane.close", &entries),
        ),
        Ok(_) => None,
        Err(error) => {
            let warning = crate::board_read_receipts::warning(error);
            tracing::warn!(%warning,window_id,"agent pane close Board observation failed open");
            None
        }
    }
}

/// This never claims process exit. The same exact historical-view finalizer is
/// used by peer close and the private stopped-Session command.
pub fn close_stopped_session(
    root: &Path,
    sessions_dir: &Path,
    expected: &gwt_agent::Session,
) -> Result<bool, String> {
    if gwt_core::paths::project_scope_hash(&expected.worktree_path)
        != gwt_core::paths::project_scope_hash(root)
    {
        return Err("Session is outside the fixed project scope".into());
    }
    crate::session_finalizer::remove_restored_stopped_view(sessions_dir, expected)
        .map_err(|error| error.to_string())
}

pub fn launch_error_terminal_bytes(detail: &str) -> Vec<u8> {
    let mut message = String::from("\r\n[gwt] Launch failed before PTY started.\r\n");
    let detail = detail.trim();
    if !detail.is_empty() {
        message.push_str("[gwt] ");
        message.push_str(detail);
        message.push_str("\r\n");
    }
    message.into_bytes()
}

pub fn manual_work_close_policy(
    lifecycle: &str,
    active_agents: usize,
    remote_unknown: bool,
) -> (bool, Option<String>) {
    let allowed = lifecycle == "paused" && active_agents == 0;
    if remote_unknown {
        return (false, Some("remote_environment_unknown".into()));
    }
    (
        allowed,
        (!allowed && !matches!(lifecycle, "done" | "discarded")).then(|| "live_agent".into()),
    )
}

pub fn retain_remote_work_close_guard(
    allowed: bool,
    reason: Option<String>,
    remote_unknown: bool,
) -> (bool, Option<String>) {
    if allowed && remote_unknown {
        (false, Some("remote_environment_unknown".into()))
    } else {
        (allowed, reason)
    }
}

pub fn validate_work_close_target(
    fresh: bool,
    cache_present: bool,
    target: Option<crate::ActiveWorkspaceWorkView>,
) -> Result<crate::ActiveWorkspaceWorkView, (String, bool)> {
    if !fresh || !cache_present {
        return Err(("Work close eligibility is unavailable or stale; refresh scheduled, retry after the Active Work view refreshes".into(),true));
    }
    let target = target.ok_or_else(|| {
        (
            "exact Work is not present in the current project Active Work view".into(),
            false,
        )
    })?;
    validate_manual_work_close(
        target.manual_close_allowed,
        target.close_blocked_reason.as_deref(),
        target
            .agents
            .iter()
            .map(|agent| agent.status_category.as_str()),
    )?;
    Ok(target)
}

fn validate_manual_work_close<'a>(
    allowed: bool,
    reason: Option<&str>,
    agent_states: impl IntoIterator<Item = &'a str>,
) -> Result<(), (String, bool)> {
    if !allowed
        || agent_states
            .into_iter()
            .any(|status| matches!(status, "active" | "running" | "blocked"))
    {
        return Err((format!("Work close refused: manual_close_allowed must be true and active_agents must be zero ({})",reason.unwrap_or("not eligible")),false));
    }
    Ok(())
}

/// Rebuild only the close facts from canonical Work membership, retained pane
/// identities and real Session observations. No UI cache or history renderer owns this decision.
pub fn fresh_work_close_eligibility(
    root: &Path,
    sessions_dir: &Path,
    work_id: &str,
    panes: &[PaneProjection],
) -> Result<(), (String, bool)> {
    use gwt_core::workspace_projection as work;
    let items = work::load_workspace_work_items(root)
        .map_err(|error| (error.to_string(), false))?
        .ok_or_else(|| ("current Work state is unavailable".into(), false))?;
    let mut exact = items.work_items.iter().filter(|item| item.id == work_id);
    let target = exact
        .next()
        .ok_or_else(|| ("Work no longer exists in this project".into(), false))?;
    if exact.next().is_some() || target.is_terminal() {
        return Err((
            "current Work identity is ambiguous or already terminal".into(),
            false,
        ));
    }
    let members = target
        .agents
        .iter()
        .map(|agent| agent.session_id.as_str())
        .collect::<HashSet<_>>();
    let registered_live = panes.iter().any(|pane| {
        pane.active.as_ref().is_some_and(|active| {
            members.contains(active.session_id.as_str())
                || work_id.strip_prefix("work-session-") == Some(active.session_id.as_str())
        })
    });
    let mut observed_or_unknown = registered_live;
    for member in &target.agents {
        match gwt_agent::Session::load(&sessions_dir.join(format!("{}.toml", member.session_id))) {
            Ok(session) => {
                let observed = crate::session_inventory::observe_session(&session, sessions_dir);
                observed_or_unknown |= !observed.sessions.is_empty()
                    || !observed.uncertainties.is_empty()
                    || matches!(
                        session.status,
                        gwt_agent::AgentStatus::Running
                            | gwt_agent::AgentStatus::Idle
                            | gwt_agent::AgentStatus::WaitingInput
                    );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err((
                    format!("Work member observation is unavailable: {error}"),
                    true,
                ))
            }
        }
    }
    let containers = target.execution_containers.iter().chain(
        target
            .events
            .iter()
            .filter_map(|event| event.execution_container.as_ref()),
    );
    let containers = containers.collect::<Vec<_>>();
    let recorded_path = containers.iter().any(|container| {
        container
            .worktree_path
            .as_ref()
            .is_some_and(|path| !path.as_os_str().is_empty())
    });
    let branches = containers
        .iter()
        .filter_map(|container| container.branch.as_deref())
        .map(|branch| {
            branch
                .trim_start_matches("refs/heads/")
                .trim_start_matches("refs/remotes/")
                .trim_start_matches("origin/")
        })
        .filter(|branch| !branch.is_empty())
        .collect::<HashSet<_>>();
    let remote_unknown = if recorded_path || branches.is_empty() {
        false
    } else {
        let main = gwt_git::worktree::main_worktree_root(root)
            .map_err(|error| (error.to_string(), true))?;
        let local = gwt_git::WorktreeManager::new(&main)
            .list()
            .map_err(|error| (error.to_string(), true))?;
        !local.iter().any(|entry| {
            entry
                .branch
                .as_deref()
                .is_some_and(|branch| branches.contains(branch))
        })
    };
    let lifecycle = if observed_or_unknown {
        "active"
    } else {
        "paused"
    };
    let (allowed, reason) =
        manual_work_close_policy(lifecycle, usize::from(observed_or_unknown), remote_unknown);
    validate_manual_work_close(allowed, reason.as_deref(), std::iter::empty())
}

pub fn pm_successor_recovery_checkout(
    project_root: &Path,
    parent: &gwt_core::workspace_projection::WorkItem,
    target: &gwt_core::workspace_projection::WorkItem,
    sessions_dir: &Path,
) -> Result<
    (
        crate::cli::execution_state::ExecutionOwnerKey,
        String,
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef,
    ),
    String,
> {
    use gwt_core::workspace_projection as work;
    let project_hash = gwt_core::paths::project_scope_hash(project_root);
    let mut containers = Vec::new();
    for item in [target, parent] {
        containers.extend(item.execution_containers.iter().cloned());
        containers.extend(
            item.events
                .iter()
                .filter_map(|event| event.execution_container.clone()),
        );
        for agent in &item.agents {
            if gwt_agent::validate_session_id_path_component(&agent.session_id).is_err() {
                return Err("Work member Session identity is invalid".into());
            }
            if let Ok(session) =
                gwt_agent::Session::load(&sessions_dir.join(format!("{}.toml", agent.session_id)))
            {
                containers.push(work::WorkspaceExecutionContainerRef {
                    branch: Some(session.branch),
                    worktree_path: Some(session.worktree_path),
                    pr_number: None,
                    pr_url: None,
                    pr_state: None,
                });
            }
        }
    }
    let mut selected = None;
    for candidate in containers {
        let Some(path) = candidate
            .worktree_path
            .as_deref()
            .and_then(|path| dunce::canonicalize(path).ok())
        else {
            continue;
        };
        let Ok(repo) = gwt_git::repository::Repository::discover(&path) else {
            continue;
        };
        if dunce::canonicalize(repo.path()).ok().as_ref() != Some(&path)
            || gwt_core::repo_hash::detect_repo_hash(&path).as_ref() != Some(&project_hash)
        {
            continue;
        }
        let Some(branch) = repo.current_branch().ok().flatten() else {
            continue;
        };
        let Some(expected_owner) = work::work_owner_for_branch(&branch) else {
            continue;
        };
        if work::canonical_work_id(project_root, Some(&branch), Some(&path))
            != work::canonical_work_id(
                project_root,
                candidate.branch.as_deref(),
                candidate.worktree_path.as_deref(),
            )
        {
            continue;
        }
        let number = expected_owner
            .strip_prefix("Issue #")
            .and_then(|number| number.parse().ok())
            .ok_or_else(|| "canonical branch owner is invalid".to_string())?;
        let container = work::WorkspaceExecutionContainerRef {
            branch: Some(branch),
            worktree_path: Some(path),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        };
        let owner = match crate::cli::execution_state::load(
            container.worktree_path.as_deref().expect("local checkout"),
        )
        .map_err(|error| format!("execution owner is unavailable: {error}"))?
        {
            Some(record) => {
                if !crate::cli::execution_state::integrity_ok(&record)
                    || record.owner_number != number
                {
                    return Err("execution owner conflicts with the canonical branch".into());
                }
                crate::cli::execution_state::ExecutionOwnerKey {
                    kind: record.owner_kind,
                    number,
                }
            }
            None => crate::cli::execution_state::ExecutionOwnerKey {
                kind: if [parent, target].iter().any(|item| {
                    item.owner.as_deref().is_some_and(|owner| {
                        owner == format!("SPEC-{number}") || owner == format!("SPEC #{number}")
                    })
                }) {
                    crate::cli::execution_state::ExecutionOwnerKind::Spec
                } else {
                    crate::cli::execution_state::ExecutionOwnerKind::Issue
                },
                number,
            },
        };
        let expected_owner = super::workspace::workspace_owner_label(owner);
        // Recovery authority comes from the PM and this proven checkout,
        // not the historical labels on the invalid successor being retired.
        let resolved = (owner, expected_owner, container);
        if selected
            .as_ref()
            .is_some_and(|previous| previous != &resolved)
        {
            return Err(
                "local owner checkout is ambiguous; workspace.close requires one exact checkout"
                    .into(),
            );
        }
        selected = Some(resolved);
    }
    selected.ok_or_else(|| {
        "cannot prove a local checkout of the canonical owner branch; workspace.close did not change Work history".into()
    })
}

pub fn record_work_close(
    project_root: &Path,
    work_id: &str,
    close_kind: gwt_core::workspace_projection::WorkCloseKind,
) -> Result<bool, String> {
    use gwt_core::workspace_projection as work;
    let changed = match close_kind {
        work::WorkCloseKind::Done => {
            work::emit_workspace_done_event_if_absent(project_root, work_id, chrono::Utc::now())
        }
        work::WorkCloseKind::Discarded => {
            work::emit_workspace_discard_event_if_absent(project_root, work_id, chrono::Utc::now())
        }
    }
    .map_err(|error| error.to_string())?;
    let items = work::load_workspace_work_items(project_root)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Work state is unavailable after close".to_string())?;
    let target = items
        .work_items
        .iter()
        .find(|item| item.id == work_id)
        .ok_or_else(|| "Work no longer exists".to_string())?;
    let matches = match close_kind {
        work::WorkCloseKind::Done => {
            target.status_category == work::WorkspaceStatusCategory::Done && !target.discarded
        }
        work::WorkCloseKind::Discarded => target.discarded,
    };
    if !matches {
        return Err(
            "Work did not reach the requested terminal state; inspect Work state before retrying"
                .into(),
        );
    }
    Ok(changed)
}

pub fn supersede_invalid_work_for_pm(
    project_root: &Path,
    sessions_dir: &Path,
    live_session_ids: &HashSet<String>,
    work_id: &str,
) -> Result<bool, (String, bool)> {
    use gwt_core::workspace_projection as work;
    let items = work::load_workspace_work_items(project_root)
        .map_err(|error| (error.to_string(), false))?
        .ok_or_else(|| ("current Work state is unavailable".into(), false))?;
    let exact_target = |items: &work::WorkItemsProjection| {
        let mut matches = items.work_items.iter().filter(|item| item.id == work_id);
        let target = matches.next().ok_or_else(|| {
            gwt_core::GwtError::Other("Work no longer exists in this project".into())
        })?;
        if matches.next().is_some() {
            return Err(gwt_core::GwtError::Other(
                    "exact Work identity is ambiguous; workspace.close cannot repair duplicate identities".into(),
                ));
        }
        Ok(target.clone())
    };
    let target = exact_target(&items).map_err(|error| (error.to_string(), false))?;
    let mut parents = items
        .work_items
        .iter()
        .filter(|item| work::successor_work_id(&item.id) == work_id);
    let parent = parents
        .next()
        .filter(|item| item.is_terminal())
        .cloned()
        .ok_or_else(|| ("an exact terminal predecessor is required".into(), false))?;
    if parents.next().is_some() {
        return Err(("exact predecessor identity is ambiguous".into(), false));
    }
    let (owner, expected_owner, container) =
        pm_successor_recovery_checkout(project_root, &parent, &target, sessions_dir)
            .map_err(|error| (error, false))?;
    let worktree = container
        .worktree_path
        .as_deref()
        .expect("recovery checkout has a proven local path");
    // Match fresh activation's global -> owner -> Work order. Acquiring
    // the owner inside the Work transaction would invert that hierarchy.
    let changed = crate::cli::execution_state::with_workspace_successor_supersession_lease(
            worktree,
            owner,
            || {
                work::transact_workspace_successor_supersession_for_work_event_root(
                    project_root,
                    worktree,
                    |_, locked, _| {
                        let locked_target = exact_target(locked)?;
                        let mut parents = locked
                            .work_items
                            .iter()
                            .filter(|item| item.id == parent.id);
                        let locked_parent = parents.next().ok_or_else(|| {
                            gwt_core::GwtError::Other("predecessor disappeared before Work supersession".into())
                        })?;
                        if parents.next().is_some()
                            || locked_parent != &parent
                            || locked_target != target
                        {
                            return Err(gwt_core::GwtError::Other(
                                "exact Work or predecessor changed; retry workspace.close".into(),
                            ));
                        }
                        let (locked_owner_key, locked_owner, locked_container) = pm_successor_recovery_checkout(
                            project_root,
                            locked_parent,
                            &locked_target,
                            sessions_dir,
                        )
                        .map_err(gwt_core::GwtError::Other)?;
                        if locked_owner_key != owner || locked_owner != expected_owner || locked_container != container {
                            return Err(gwt_core::GwtError::Other(
                                "local owner checkout changed before Work supersession".into(),
                            ));
                        }
                        if sessions_dir != gwt_core::paths::gwt_sessions_dir() {
                            super::workspace::validate_superseded_member_runtimes(
                                sessions_dir,
                                &locked_target,
                            )?;
                        }
                        let audit = super::workspace::invalid_successor_supersession_event(
                            project_root,
                            locked_parent,
                            &locked_target,
                            container.clone(),
                            super::workspace::SuccessorSupersessionAuthority::RegisteredPm,
                            live_session_ids,
                            chrono::Utc::now(),
                        )?;
                        if locked_target.events.iter().any(|event| {
                            event.work_item_id == locked_target.id
                                && event.is_successor_supersession()
                        }) {
                            return Ok((false, Vec::new()));
                        }
                        let current = work::current_work_id_for_launch(
                            locked,
                            project_root,
                            container.branch.as_deref(),
                            container.worktree_path.as_deref(),
                            // Diagnose structural occupancy independently of
                            // the foreign owner this PM recovery is repairing.
                            None,
                        )?;
                        if current.as_deref() != Some(locked_parent.id.as_str()) {
                            return Err(gwt_core::GwtError::Other(
                                "Work already belongs to a valid successor lineage; use issue.monitor.requeue".into(),
                            ));
                        }
                        Ok((true, vec![audit]))
                    },
                )
                .map_err(std::io::Error::other)
            },
        )
        .map_err(|error| (error.to_string(), false))?;
    Ok(changed)
}

#[allow(clippy::too_many_arguments)]
/// Native UI requests already hold the private fixed-project transport.
/// Evaluate current owner membership rather than a GUI active-session cache.
pub fn close_work_from_native(
    root: &Path,
    sessions_dir: &Path,
    work_id: &str,
    close_kind: &str,
    live_session_ids: &HashSet<String>,
) -> Result<bool, String> {
    use gwt_core::workspace_projection as work;
    let kind = match close_kind {
        "done" => work::WorkCloseKind::Done,
        "discarded" => work::WorkCloseKind::Discarded,
        _ => return Err("close_kind must be done or discarded".into()),
    };
    let items = work::load_workspace_work_items(root).map_err(|error| error.to_string())?;
    let target = items
        .as_ref()
        .and_then(|items| items.work_items.iter().find(|item| item.id == work_id));
    let mut members: HashSet<_> = target
        .into_iter()
        .flat_map(|item| item.agents.iter().map(|agent| agent.session_id.clone()))
        .collect();
    if let Some(session) = work_id
        .strip_prefix("work-session-")
        .filter(|session| !session.is_empty())
    {
        members.insert(session.into());
    }
    let mut has_live_agent = members
        .iter()
        .any(|session| live_session_ids.contains(session));
    for id in members {
        let path = sessions_dir.join(format!("{id}.toml"));
        if path.exists() {
            let session = gwt_agent::Session::load(&path).map_err(|error| error.to_string())?;
            has_live_agent |= !crate::session_inventory::observe_session(&session, sessions_dir)
                .sessions
                .is_empty();
        }
    }
    if work::decide_work_close(has_live_agent, None) == work::WorkCloseDecision::BlockedLiveAgent {
        return Err(
            "Work close blocked: owning agent session is still live; stop the agent before closing"
                .into(),
        );
    }
    record_work_close(root, work_id, kind)
}

pub struct AgentWorkCloseRequest<'a> {
    pub work_id: &'a str,
    pub close_kind: &'a str,
}

pub fn close_work_for_agent(
    root: &Path,
    sessions_dir: &Path,
    panes: &[PaneProjection],
    live_session_ids: &HashSet<String>,
    principal: &AgentSessionPrincipal,
    request: AgentWorkCloseRequest<'_>,
    eligibility: impl FnOnce() -> Result<(), (String, bool)>,
) -> Result<bool, (String, bool)> {
    let AgentWorkCloseRequest {
        work_id,
        close_kind,
    } = request;
    use gwt_core::workspace_projection as work;
    let kind = match close_kind {
        "done" => Some(work::WorkCloseKind::Done),
        "discarded" => Some(work::WorkCloseKind::Discarded),
        "superseded" => None,
        _ => {
            return Err((
                "close_kind must be done, discarded, or superseded".into(),
                false,
            ))
        }
    };
    if work_id.is_empty() || work_id.trim() != work_id {
        return Err(("an exact nonempty work_id is required".into(), false));
    }
    if !principal.authorizes_project_root(root) {
        return Err(("authenticated project is not open".into(), false));
    }
    let window_id = principal_session_window(panes, principal).map_err(|error| (error, false))?;
    if !crate::pm_registry::session_is_registered_pm(
        &crate::pm_registry::pm_prefs_path_for_repo_path(root),
        principal.session_id(),
    ) || live_pm_window_id(panes, principal.session_id()).as_deref() != Some(window_id.as_str())
    {
        return Err((
            "workspace.close requires the live registered PM of this project".into(),
            false,
        ));
    }
    let Some(kind) = kind else {
        return supersede_invalid_work_for_pm(root, sessions_dir, live_session_ids, work_id);
    };
    let items = work::load_workspace_work_items(root)
        .map_err(|error| (error.to_string(), false))?
        .ok_or_else(|| ("current Work state is unavailable".into(), false))?;
    let mut exact = items.work_items.iter().filter(|item| item.id == work_id);
    let target = exact
        .next()
        .ok_or_else(|| ("Work no longer exists in this project".into(), false))?;
    if exact.next().is_some() || target.is_terminal() {
        return Err((
            "current Work identity is ambiguous or already terminal".into(),
            false,
        ));
    }
    if live_session_ids.iter().any(|session_id| {
        target
            .agents
            .iter()
            .any(|agent| &agent.session_id == session_id)
            || work_id.strip_prefix("work-session-") == Some(session_id.as_str())
    }) {
        return Err((
            "Work close refused: a current member agent is live; stop it before closing".into(),
            false,
        ));
    }
    eligibility()?;
    record_work_close(root, work_id, kind).map_err(|error| (error, false))
}

pub fn pane_owner_number(pane: &PaneProjection, sessions_dir: &Path) -> Option<u64> {
    pane.window
        .linked_issue_number
        .or_else(|| {
            crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(
                &pane.project_root,
            ))
            .ok()
            .and_then(|prefs| {
                prefs
                    .launched_issues
                    .into_iter()
                    .find(|row| {
                        row.window_id == pane.window.id
                            || (!row.window_id.contains("::")
                                && pane
                                    .window
                                    .id
                                    .split_once("::")
                                    .is_some_and(|(_, raw)| raw == row.window_id))
                    })
                    .map(|row| row.issue_number)
            })
        })
        .or_else(|| {
            pane.window
                .session_id
                .as_deref()
                .and_then(|id| {
                    gwt_agent::Session::load(&sessions_dir.join(format!("{id}.toml"))).ok()
                })
                .and_then(|session| session.linked_issue_number)
        })
}

pub fn peer_close_precondition(
    panes: &[PaneProjection],
    sessions_dir: &Path,
    principal: &AgentSessionPrincipal,
    id: &str,
) -> Result<(), String> {
    let pane = panes
        .iter()
        .find(|pane| pane.window.id == id && principal.authorizes_project_root(&pane.project_root))
        .ok_or("window is outside the authenticated project scope")?;
    if pane.window.session_id.as_deref() == Some(principal.session_id()) {
        return Err(
            "closing the calling session requires a correlated acceptance (request_id)".into(),
        );
    }
    let owner = pane_owner_number(pane, sessions_dir);
    if let Some(reason) = agent_close_board_check(
        &pane.project_root,
        principal,
        pane.window.session_id.as_deref(),
        owner,
        id,
    ) {
        return Err(reason);
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub struct FleetPhysicalProof {
    pub runtime_present: bool,
    pub child_pid: Option<u32>,
    pub child_started_at: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
pub fn close_fleet_pane<T>(
    panes: &[PaneProjection],
    sessions_dir: &Path,
    principal: &AgentSessionPrincipal,
    id: &str,
    session_id: &str,
    child_pid: Option<u32>,
    child_started_at: Option<u64>,
    actual: FleetPhysicalProof,
    close: impl FnOnce() -> T,
) -> Result<T, String> {
    let pane = panes
        .iter()
        .find(|pane| pane.window.id == id && principal.authorizes_project_root(&pane.project_root))
        .ok_or("window is outside the authenticated project scope")?;
    let registered = crate::pm_registry::load_pm_prefs(
        &crate::pm_registry::pm_prefs_path_for_repo_path(principal.canonical_project_root()),
    )
    .is_ok_and(|prefs| {
        prefs.registration.is_some_and(|registration| {
            registration.session_id == principal.session_id()
                && live_pm_window_id(panes, principal.session_id()).is_some()
        })
    });
    if !registered {
        return Err("fleet close requires the live registered PM".into());
    }
    if session_id.is_empty() || pane.window.session_id.as_deref() != Some(session_id) {
        return Err("window session changed since fleet planning".into());
    }
    let matches = match (actual.runtime_present, child_pid, child_started_at) {
        (false, None, None) => true,
        (true, Some(pid), Some(started)) => {
            actual.child_pid == Some(pid)
                && crate::process::host_process_start_time(pid) == Some(started)
        }
        _ => false,
    };
    if !matches {
        return Err("window runtime changed or is unavailable; refresh fleet inventory".into());
    }
    if session_id == principal.session_id() {
        return Err("fleet close cannot close the calling PM session".into());
    }
    let owner = pane_owner_number(pane, sessions_dir);
    if let Some(reason) =
        agent_close_board_check(&pane.project_root, principal, Some(session_id), owner, id)
    {
        return Err(reason);
    }
    let worktree = pane
        .active
        .as_ref()
        .filter(|active| active.session_id == session_id)
        .map(|active| active.worktree_path.clone())
        .or_else(|| {
            gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml")))
                .ok()
                .map(|session| session.worktree_path)
        })
        .unwrap_or_else(|| principal.canonical_project_root().into());
    let prefs_path =
        crate::issue_monitor_prefs_path_for_repo_path(principal.canonical_project_root());
    crate::cli::execution_state::with_fleet_cleanup_lease(
        &worktree,
        sessions_dir,
        Some(session_id),
        || {
            let _claim = gwt_core::operation_deadline::NamedFileLock::try_acquire(
                &prefs_path.with_extension("lock"),
                "fleet.close",
            )?;
            let prefs = crate::load_issue_monitor_prefs(&prefs_path)?;
            let same = |candidate: &str| {
                candidate == id
                    || (!candidate.contains("::")
                        && id.split_once("::").is_some_and(|(_, raw)| raw == candidate))
            };
            if prefs.launched_issues.iter().any(|row| same(&row.window_id))
                || prefs
                    .launch_confirmations
                    .iter()
                    .any(|(owner, confirmation)| {
                        same(&confirmation.window_id)
                            && confirmation.claim_id.as_ref().is_some_and(|claim| {
                                prefs.launched_claims.get(owner) == Some(claim)
                            })
                    })
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "current claim holder is protected; use its owner lifecycle operation",
                ));
            }
            Ok(close())
        },
    )
    .map_err(|error| {
        format!("fleet close retained target: {error}; refresh fleet inventory before retry")
    })
}

pub struct RestoredRecoveryIdentity<'a> {
    pub session_id: &'a str,
    pub child_pid: u32,
    pub child_started_at: u64,
}

pub fn validate_restored_recovery(
    panes: &[PaneProjection],
    sessions_dir: &Path,
    principal: &AgentSessionPrincipal,
    id: &str,
    identity: RestoredRecoveryIdentity<'_>,
    actual_pid: Option<u32>,
) -> Result<(), String> {
    let RestoredRecoveryIdentity {
        session_id,
        child_pid,
        child_started_at,
    } = identity;
    let pane = panes
        .iter()
        .find(|pane| pane.window.id == id && principal.authorizes_project_root(&pane.project_root))
        .ok_or("window is outside the authenticated project scope")?;
    if pane.window.session_id.as_deref() != Some(session_id) {
        return Err("window session changed since recovery was planned".into());
    }
    let restored = gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml")))
        .is_ok_and(|session| {
            session.id == session_id
                && session.launch_origin == gwt_agent::SessionLaunchOrigin::AutomaticRestore
                && session
                    .restore_source_session_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
        });
    if !restored {
        return Err("window session is not a proven automatic restore".into());
    }
    if actual_pid != Some(child_pid)
        || crate::process::host_process_start_time(child_pid) != Some(child_started_at)
    {
        return Err("window process changed or is unavailable; refresh recovery candidates".into());
    }
    peer_close_precondition(panes, sessions_dir, principal, id)
}

#[derive(Clone)]
pub struct PendingAgentSelfClose {
    pub ticket: crate::agent_capability::AgentSelfCloseCapabilityTicket,
    pub window_id: String,
    pub tab_id: String,
    pub raw_id: String,
    pub session_id: String,
    pub window_lifecycle_generation: u64,
}

#[allow(clippy::too_many_arguments)]
pub fn begin_self_close(
    panes: &[PaneProjection],
    sessions_dir: &Path,
    issuer: &AgentCapabilityIssuer,
    grant: &AgentCapabilityGrant,
    requested_id: &str,
    request_id: &str,
    window_generation: Option<u64>,
) -> Result<PendingAgentSelfClose, String> {
    if !uuid::Uuid::parse_str(request_id)
        .ok()
        .is_some_and(|id| id.hyphenated().to_string() == request_id)
    {
        return Err("agent self-close rejected an invalid correlation id".into());
    }
    let own = principal_session_window(panes, grant.principal())?;
    if requested_id != own {
        return Err("correlated agent self-close targeted a peer or foreign window".into());
    }
    let pane = panes
        .iter()
        .find(|pane| pane.window.id == own)
        .ok_or("agent self-close Session changed before acceptance")?;
    let session = pane
        .window
        .session_id
        .as_deref()
        .filter(|id| *id == grant.principal().session_id())
        .ok_or("agent self-close Session changed before acceptance")?;
    let owner = pane_owner_number(pane, sessions_dir);
    if let Some(reason) = agent_close_board_check(
        &pane.project_root,
        grant.principal(),
        Some(session),
        owner,
        &own,
    ) {
        return Err(reason);
    }
    let generation = window_generation
        .ok_or("agent self-close window generation was unavailable at acceptance")?;
    let ticket = issuer
        .begin_self_close_if_current(grant)
        .ok_or("agent self-close capability changed before acceptance")?;
    Ok(PendingAgentSelfClose {
        ticket,
        window_id: own,
        tab_id: pane.tab_id.clone(),
        raw_id: pane
            .window
            .id
            .strip_prefix(&format!("{}::", pane.tab_id))
            .unwrap_or(&pane.window.id)
            .into(),
        session_id: session.into(),
        window_lifecycle_generation: generation,
    })
}

pub fn fleet_pane_inventory(
    principal: &AgentSessionPrincipal,
    panes: &[PaneProjection],
    physical: &HashMap<String, FleetPhysicalProof>,
    sessions: &HashMap<String, gwt_agent::Session>,
) -> Vec<crate::protocol::FleetPaneView> {
    panes
        .iter()
        .filter(|pane| {
            principal.authorizes_project_root(&pane.project_root)
                && pane.window.preset.requires_process()
        })
        .map(|pane| {
            let session = pane
                .window
                .session_id
                .as_ref()
                .and_then(|id| sessions.get(id));
            let proof = physical.get(&pane.window.id);
            let path = pane
                .active
                .as_ref()
                .filter(|active| {
                    Some(active.session_id.as_str()) == pane.window.session_id.as_deref()
                })
                .map(|active| &active.worktree_path)
                .or_else(|| session.map(|session| &session.worktree_path))
                .unwrap_or(&pane.project_root);
            crate::protocol::FleetPaneView {
                window_id: pane.window.id.clone(),
                project_root: pane.project_root.display().to_string(),
                session_id: pane.window.session_id.clone(),
                runtime_present: proof.is_some_and(|proof| proof.runtime_present),
                child_pid: proof.and_then(|proof| proof.child_pid),
                child_started_at: proof.and_then(|proof| proof.child_started_at),
                title: pane.window.title.clone(),
                state: pane.window.status,
                agent_name: pane.window.agent_name.clone(),
                preset: pane.window.preset,
                worktree_path: path.display().to_string(),
                linked_issue_number: pane
                    .window
                    .linked_issue_number
                    .or_else(|| session.and_then(|session| session.linked_issue_number)),
                is_pm: crate::pm_registry::session_is_registered_pm(
                    &crate::pm_registry::pm_prefs_path_for_repo_path(&pane.project_root),
                    pane.window.session_id.as_deref().unwrap_or_default(),
                ),
            }
        })
        .collect()
}

/// Execute one physical write while the exact registered incarnation is current.
/// The registry read guard is never held during parser access or submit settling.
fn with_current_writer<T>(
    writers: &PtyWriterRegistry,
    id: &str,
    entry: &Arc<PtyWriterEntry>,
    write: impl FnOnce(&Arc<PtyHandle>) -> Result<T, String>,
) -> Result<T, String> {
    let registered = writers.read().map_err(|error| error.to_string())?;
    if !registered
        .get(id)
        .is_some_and(|current| Arc::ptr_eq(current, entry))
        || !entry.current.load(std::sync::atomic::Ordering::Acquire)
    {
        return Err("pane runtime changed before input commit".into());
    }
    drop(registered);
    write(&entry.handle)
}

/// The private GUI and agent commands use the same ordinary-input revision.
/// A captured entry cannot target a same-address replacement.
pub fn write_current_raw_input(
    writers: &PtyWriterRegistry,
    id: &str,
    entry: &Arc<PtyWriterEntry>,
    bytes: &[u8],
) -> Result<(), String> {
    with_current_writer(writers, id, entry, |handle| {
        handle.write_input(bytes).map_err(|error| error.to_string())
    })
}

/// Synchronous accepted prompt delivery; the final CR stays inside its grant
/// and durable Session lease, and rechecks the physical incarnation after settle.
pub fn write_current_pane_input(
    writers: &PtyWriterRegistry,
    id: &str,
    entry: &Arc<PtyWriterEntry>,
    text: &str,
) -> Result<(), String> {
    let (body, submit) = split_pane_submit(text);
    let Some(submit) = submit else {
        return if body.is_empty() {
            Ok(())
        } else {
            write_current_raw_input(writers, id, entry, body.as_bytes())
        };
    };
    let reservation = with_current_writer(writers, id, entry, |handle| {
        Arc::clone(handle)
            .reserve_input_transaction()
            .map_err(|error| error.to_string())
    })?;
    if !body.is_empty() {
        let bytes = pane_input_body_bytes(body, reservation.bracketed_paste_enabled())?;
        with_current_writer(writers, id, entry, |_| {
            reservation
                .write_input_authorized(&bytes, |commit| {
                    if !entry.current.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(gwt_terminal::TerminalError::PtyIoError {
                            details: "pane runtime changed before body commit".into(),
                        });
                    }
                    commit(&mut || {})
                })
                .map_err(|error| error.to_string())
        })?;
    }
    thread::sleep(PANE_SUBMIT_SETTLE);
    with_current_writer(writers, id, entry, |_| {
        reservation
            .write_input_authorized(submit.as_bytes(), |commit| {
                if !entry.current.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(gwt_terminal::TerminalError::PtyIoError {
                        details: "pane runtime changed before submit commit".into(),
                    });
                }
                commit(&mut || {})
            })
            .map_err(|error| error.to_string())
    })
}

/// Called by a close worker, never by the mailbox while another write holds
/// the read lock. Failed retirement keeps its exact runtime/current fence.
pub(crate) fn invalidate_current_writer(
    writers: &PtyWriterRegistry,
    id: &str,
    entry: &Arc<PtyWriterEntry>,
) -> Result<(), String> {
    let mut registered = writers.write().map_err(|error| error.to_string())?;
    if !registered
        .get(id)
        .is_some_and(|current| Arc::ptr_eq(current, entry))
    {
        return Err("pane writer incarnation changed before close".into());
    }
    entry
        .current
        .store(false, std::sync::atomic::Ordering::Release);
    entry.handle.revoke_input_generation();
    registered.remove(id);
    Ok(())
}

/// Captured from the single owner's registry and physical runtimes. It contains
/// no GUI writer, callback authority, or independently admitted runtime.
pub(crate) struct AgentDispatchCapture {
    pub root: PathBuf,
    pub registry_path: PathBuf,
    pub sessions_dir: PathBuf,
    pub issuer: AgentCapabilityIssuer,
    pub clients: crate::project_transport::ClientHub,
    pub panes: Vec<PaneProjection>,
    pub snapshots: Vec<PaneSnapshotInput>,
    pub workspace: ProjectTabView,
    pub physical: HashMap<String, FleetPhysicalProof>,
    pub generations: HashMap<String, u64>,
    pub writers: PtyWriterRegistry,
    pub registry: Arc<Mutex<crate::WindowRegistry>>,
}

pub(crate) struct AgentDispatchCompletion {
    pub client_id: String,
    pub outcome: super::agent_frontend::AgentFrontendDispatchOutcome,
    pub close: Option<AgentCloseCompletion>,
    pub acceptance: Option<AgentSelfCloseAcceptance>,
    pub invalidated: bool,
}

pub(crate) struct AgentSelfCloseAcceptance {
    pub pending: PendingAgentSelfClose,
    pub responder: crate::agent_capability::AgentSelfCloseResponder,
    pub request_id: String,
}

pub(crate) struct AgentCloseCompletion {
    pub expected_session_id: Option<String>,
    pub husk: Option<StoppedWindowRetirement>,
    pub id: String,
    pub runtime: Option<super::owner::WindowRuntime>,
    pub accepted: bool,
    pub result: Result<(), String>,
}

impl AgentDispatchCapture {
    pub(crate) fn run(
        self,
        client_id: String,
        grant: AgentCapabilityGrant,
        request: crate::agent_capability::AgentFrontendRequest,
        close: Option<AgentCloseCompletion>,
    ) -> AgentDispatchCompletion {
        use super::agent_frontend::{dispatch_if_current, AcceptedAgentRequest};
        #[cfg(any(test, feature = "test-gh-guard"))]
        let _trace = super::agent_frontend::enter_owner_agent_trace(&grant, &request);
        use crate::agent_capability::AgentFrontendRequest;
        // Name's machine-local guard always precedes ECR/Session and grant
        // locks. It stays held through the durable row publication below.
        let prepared_name = matches!(request, AgentFrontendRequest::Name { .. })
            .then(|| prepare_pane_name(&self.panes, grant.principal(), &self.sessions_dir));
        let mut completion = AgentDispatchCompletion {
            client_id: client_id.clone(),
            outcome: super::agent_frontend::AgentFrontendDispatchOutcome::Dispatched(Vec::new()),
            close,
            acceptance: None,
            invalidated: false,
        };
        let issuer = self.issuer.clone();
        let sessions_dir = self.sessions_dir.clone();
        completion.outcome = dispatch_if_current(
            Some(&issuer),
            &sessions_dir,
            client_id,
            grant,
            request,
            |client_id, accepted| match accepted {
                AcceptedAgentRequest::SelfClose {
                    grant,
                    id,
                    request_id,
                    responder,
                } => {
                    match begin_self_close(
                        &self.panes,
                        &self.sessions_dir,
                        &issuer,
                        &grant,
                        &id,
                        &request_id,
                        self.generations.get(&id).copied(),
                    ) {
                        Ok(pending) => {
                            completion.acceptance = Some(AgentSelfCloseAcceptance {
                                pending,
                                responder,
                                request_id,
                            })
                        }
                        Err(reason) => {
                            responder.refuse(id, reason);
                        }
                    }
                    Vec::new()
                }
                AcceptedAgentRequest::PmSend {
                    grant,
                    operation_id,
                    window_id,
                    text,
                    responder,
                } => {
                    let clients = self.clients.clone();
                    authenticated_pm_pane_send_input_events(
                        &issuer,
                        client_id,
                        grant,
                        &operation_id,
                        &window_id,
                        &text,
                        responder,
                        self.panes.clone(),
                        &self.writers,
                        |task| {
                            task();
                            Ok(())
                        },
                        Arc::new(move |event| clients.dispatch(vec![event])),
                    )
                }
                AcceptedAgentRequest::Principal { principal, request } => {
                    match request {
                        AgentFrontendRequest::Ready => {
                            let mut workspace = self.workspace;
                            for window in &mut workspace.workspace.windows {
                                window.worktree_form = self.panes.iter()
                                    .find(|pane| pane.window.id == window.id)
                                    .and_then(|pane| pane.active.as_ref())
                                    .map(|active| {
                                        if super::workspace::session_uses_ephemeral_worktree_for_project(Some(&self.root), active) {
                                            crate::WindowWorktreeForm::Ephemeral
                                        } else {
                                            crate::WindowWorktreeForm::BranchBacked
                                        }
                                    })
                                    .unwrap_or(crate::WindowWorktreeForm::Unknown);
                            }
                            let event = workspace_state_reply(
                                &client_id,
                                &principal,
                                env!("CARGO_PKG_VERSION").into(),
                                vec![(self.root.clone(), workspace)],
                            );
                            frontend_sync_events(&client_id, event, self.snapshots)
                        }
                        AgentFrontendRequest::ListWindows => vec![workspace_state_reply(
                            &client_id,
                            &principal,
                            env!("CARGO_PKG_VERSION").into(),
                            vec![(self.root.clone(), self.workspace)],
                        )],
                        AgentFrontendRequest::ListFleetPanes => {
                            let sessions = self
                                .panes
                                .iter()
                                .filter_map(|pane| {
                                    let id = pane.window.session_id.as_ref()?;
                                    gwt_agent::Session::load(
                                        &self.sessions_dir.join(format!("{id}.toml")),
                                    )
                                    .ok()
                                    .map(|session| (id.clone(), session))
                                })
                                .collect();
                            vec![OutboundEvent::reply(
                                client_id,
                                BackendEvent::FleetPaneList {
                                    project_root: principal
                                        .canonical_project_root()
                                        .display()
                                        .to_string(),
                                    panes: fleet_pane_inventory(
                                        &principal,
                                        &self.panes,
                                        &self.physical,
                                        &sessions,
                                    ),
                                },
                            )]
                        }
                        AgentFrontendRequest::Name { request_id, .. } => {
                            let result = (|| {
                                let prepared = prepared_name
                                    .as_ref()
                                    .expect("Name prepared before gate")
                                    .as_ref()
                                    .map_err(Clone::clone)?;
                                let entry = self
                                    .writers
                                    .read()
                                    .map_err(|error| error.to_string())?
                                    .get(prepared.window_id())
                                    .cloned()
                                    .ok_or("Agent window is missing its physical writer")?;
                                with_current_writer(
                                    &self.writers,
                                    prepared.window_id(),
                                    &entry,
                                    |_| {
                                        let assigned = assign_prepared_pane_name(
                                            &self.panes,
                                            &self.sessions_dir,
                                            prepared,
                                        )?;
                                        let mut registry = self
                                            .registry
                                            .lock()
                                            .map_err(|error| error.to_string())?;
                                        // The row still has to identify the exact captured Session.
                                        let captured = self
                                            .panes
                                            .iter()
                                            .find(|pane| pane.window.id == assigned.window_id)
                                            .ok_or("Agent window is missing")?;
                                        let raw =
                                            raw_owner_window_id(&self.root, &captured.window.id)?;
                                        if registry
                                            .window(raw)
                                            .and_then(|row| row.session_id.as_deref())
                                            != captured.window.session_id.as_deref()
                                        {
                                            return Err(
                                                "Agent window changed before name publication"
                                                    .into(),
                                            );
                                        }
                                        registry.set_agent_name(raw, Some(assigned.name.clone()));
                                        crate::persistence::save_workspace_state_durable(
                                            &crate::persistence::workspace_state_path(&self.root),
                                            &registry.persistable_state(),
                                        )
                                        .map_err(|error| error.to_string())?;
                                        Ok(assigned)
                                    },
                                )
                            })();
                            completion.invalidated = result
                                .as_ref()
                                .is_ok_and(|assigned| !assigned.already_named);
                            let (ok, agent_name, error) = match result {
                                Ok(assigned) => (true, Some(assigned.name), None),
                                Err(error) => (false, None, Some(error)),
                            };
                            vec![OutboundEvent::reply(
                                client_id,
                                BackendEvent::PaneNameResult {
                                    request_id,
                                    ok,
                                    agent_name,
                                    error,
                                },
                            )]
                        }
                        AgentFrontendRequest::SendInput { text } => {
                            let target = principal_session_window(&self.panes, &principal);
                            let result = target.as_ref().map_err(Clone::clone).and_then(|id| {
                                let entry = self
                                    .writers
                                    .read()
                                    .map_err(|error| error.to_string())?
                                    .get(id)
                                    .cloned()
                                    .ok_or_else(|| format!("no live runtime for pane {id}"))?;
                                write_current_pane_input(&self.writers, id, &entry, &text)
                            });
                            vec![OutboundEvent::reply(
                                client_id,
                                BackendEvent::PaneSendResult {
                                    ok: result.is_ok(),
                                    window_id: target.ok(),
                                    error: result.err(),
                                },
                            )]
                        }
                        AgentFrontendRequest::CloseWork {
                            work_id,
                            close_kind,
                        } => {
                            let live = self
                                .panes
                                .iter()
                                .filter_map(|pane| pane.active.as_ref())
                                .map(|active| active.session_id.clone())
                                .collect();
                            let result = close_work_for_agent(
                                &self.root,
                                &self.sessions_dir,
                                &self.panes,
                                &live,
                                &principal,
                                AgentWorkCloseRequest {
                                    work_id: &work_id,
                                    close_kind: &close_kind,
                                },
                                || {
                                    fresh_work_close_eligibility(
                                        &self.root,
                                        &self.sessions_dir,
                                        &work_id,
                                        &self.panes,
                                    )
                                },
                            );
                            completion.invalidated = result.as_ref().is_ok_and(|changed| *changed);
                            let (ok, changed, retryable, error) = match result {
                                Ok(changed) => (true, Some(changed), false, None),
                                Err((error, retryable)) => (false, None, retryable, Some(error)),
                            };
                            vec![OutboundEvent::reply(
                                client_id,
                                BackendEvent::WorkCloseResult {
                                    work_id,
                                    close_kind,
                                    ok,
                                    changed,
                                    retryable,
                                    error,
                                },
                            )]
                        }
                        request @ (AgentFrontendRequest::CloseWindow { .. }
                        | AgentFrontendRequest::CloseFleetPane { .. }
                        | AgentFrontendRequest::RecoverRestoredWindow { .. }) => {
                            self.close_request(client_id, &principal, request, &mut completion)
                        }
                        AgentFrontendRequest::IssueMonitorScanNow {
                            expected_project_scope,
                        } => issue_monitor_scan_now_events(
                            &self.root,
                            &self.panes,
                            self.workspace.kind == crate::ProjectKind::Git,
                            client_id,
                            &principal,
                            &expected_project_scope,
                        ),
                        AgentFrontendRequest::PmSendInput { .. } => Vec::new(),
                    }
                }
            },
        );
        completion
    }
}

fn raw_owner_window_id<'a>(root: &Path, id: &'a str) -> Result<&'a str, String> {
    let key = gwt_core::paths::resolve_project_scope(root).hash;
    id.strip_prefix(&format!("{}::", key.as_str()))
        .filter(|raw| !raw.is_empty())
        .ok_or_else(|| "window is outside the fixed project scope".into())
}

impl AgentDispatchCapture {
    fn close_request(
        &self,
        client_id: String,
        principal: &AgentSessionPrincipal,
        request: crate::agent_capability::AgentFrontendRequest,
        completion: &mut AgentDispatchCompletion,
    ) -> Vec<OutboundEvent> {
        use crate::agent_capability::AgentFrontendRequest;
        let (id, fleet_request) = match &request {
            AgentFrontendRequest::CloseWindow { id, .. }
            | AgentFrontendRequest::RecoverRestoredWindow { id, .. } => (id.clone(), None),
            AgentFrontendRequest::CloseFleetPane { id, request_id, .. } => {
                (id.clone(), Some(request_id.clone()))
            }
            _ => unreachable!("typed close request"),
        };
        let actual = self
            .physical
            .get(&id)
            .copied()
            .unwrap_or(FleetPhysicalProof {
                runtime_present: false,
                child_pid: None,
                child_started_at: None,
            });
        // Capture a stopped view before taking the fleet lease. Session view
        // persistence must run after that lease is released, not nested in it.
        if let Some(captured) = completion
            .close
            .as_mut()
            .filter(|close| close.runtime.is_none())
        {
            let expected = match captured.expected_session_id.as_ref() {
                Some(session_id) => match gwt_agent::Session::load(
                    &self.sessions_dir.join(format!("{session_id}.toml")),
                ) {
                    Ok(expected) => Some(expected),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => {
                        return close_reply(&client_id, &id, fleet_request, error.to_string())
                    }
                },
                None => None,
            };
            captured.husk = Some(StoppedWindowRetirement {
                sessions_dir: self.sessions_dir.clone(),
                expected,
                cleanup: OwnedWindowRecordCleanup {
                    registry: self.registry.clone(),
                    registry_path: self.registry_path.clone(),
                    root: self.root.clone(),
                    project_key: self.workspace.id.clone(),
                    canonical_id: id.clone(),
                    expected_session_id: captured.expected_session_id.clone(),
                },
            });
        }
        let mut accept = || -> Result<(), String> {
            let captured = completion
                .close
                .as_mut()
                .ok_or("close target changed before dispatch")?;
            if captured.id != id {
                return Err("close target changed before dispatch".into());
            }
            captured.accepted = true;
            if let Some(runtime) = captured.runtime.as_ref() {
                runtime
                    .current
                    .store(false, std::sync::atomic::Ordering::Release);
                runtime.pty.revoke_input_generation();
            }
            Ok(())
        };
        let authorized = match request {
            AgentFrontendRequest::CloseWindow {
                request_id: None,
                responder: None,
                ..
            } => peer_close_precondition(&self.panes, &self.sessions_dir, principal, &id)
                .and_then(|_| accept()),
            AgentFrontendRequest::CloseWindow { .. } => {
                Err("a correlated self-close was not routed with its response channel".into())
            }
            AgentFrontendRequest::RecoverRestoredWindow {
                session_id,
                child_pid,
                child_started_at,
                ..
            } => validate_restored_recovery(
                &self.panes,
                &self.sessions_dir,
                principal,
                &id,
                RestoredRecoveryIdentity {
                    session_id: &session_id,
                    child_pid,
                    child_started_at,
                },
                actual.child_pid,
            )
            .and_then(|_| accept()),
            AgentFrontendRequest::CloseFleetPane {
                session_id,
                child_pid,
                child_started_at,
                ..
            } => close_fleet_pane(
                &self.panes,
                &self.sessions_dir,
                principal,
                &id,
                &session_id,
                child_pid,
                child_started_at,
                actual,
                accept,
            )
            .and_then(std::convert::identity),
            _ => unreachable!(),
        };
        let result = authorized.and_then(|()| {
            let captured = completion.close.as_mut().expect("accepted exact close");
            let result = if let Some(runtime) = captured.runtime.as_mut() {
                // All fleet cleanup leases are now released. Teardown retains
                // the same captured physical handle and revalidates its Session.
                let entry = self
                    .writers
                    .read()
                    .map_err(|error| error.to_string())?
                    .get(&id)
                    .cloned()
                    .ok_or("close target lost its physical writer")?;
                invalidate_current_writer(&self.writers, &id, &entry)?;
                super::owner::close_runtime(&id, runtime, super::owner::CloseCause::ExplicitClose)
            } else {
                captured
                    .husk
                    .as_mut()
                    .expect("captured stopped view")
                    .finish()
            };
            result
        });
        if let Some(captured) = completion.close.as_mut().filter(|close| close.accepted) {
            captured.result = result.clone();
        }
        completion.invalidated = completion
            .close
            .as_ref()
            .is_some_and(|close| close.accepted);
        let historical_husk = completion.close.as_ref().is_some_and(|close| {
            close.runtime.is_none()
                && close
                    .husk
                    .as_ref()
                    .is_some_and(|husk| husk.expected.is_some())
        });
        let ok = result.is_ok();
        let reason = result.err();
        vec![OutboundEvent::reply(
            client_id,
            match fleet_request {
                Some(request_id) => BackendEvent::FleetPaneCloseResult {
                    request_id,
                    ok,
                    window_id: id,
                    reason,
                },
                None if ok && historical_husk => BackendEvent::PaneHuskRemoved { window_id: id },
                None => BackendEvent::PaneCloseResult {
                    ok,
                    window_id: id,
                    reason,
                },
            },
        )]
    }
}

fn close_reply(
    client_id: &str,
    id: &str,
    fleet_request: Option<String>,
    reason: String,
) -> Vec<OutboundEvent> {
    vec![OutboundEvent::reply(
        client_id,
        match fleet_request {
            Some(request_id) => BackendEvent::FleetPaneCloseResult {
                request_id,
                ok: false,
                window_id: id.into(),
                reason: Some(reason),
            },
            None => BackendEvent::PaneCloseResult {
                ok: false,
                window_id: id.into(),
                reason: Some(reason),
            },
        },
    )]
}

pub fn issue_monitor_scope_matches(expected: &str, actual: &str) -> bool {
    expected.len() == actual.len()
        && expected
            .as_bytes()
            .iter()
            .zip(actual.as_bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

pub fn issue_monitor_scan_permission(
    root: &Path,
    panes: &[PaneProjection],
    project_available: bool,
    principal: &AgentSessionPrincipal,
    expected_scope: &str,
) -> Result<(), &'static str> {
    if !issue_monitor_scope_matches(
        expected_scope,
        gwt_core::paths::project_scope_hash(principal.canonical_project_root()).as_str(),
    ) {
        return Err("project_scope_mismatch");
    }
    let registered = crate::pm_registry::load_pm_prefs(
        &crate::pm_registry::pm_prefs_path_for_repo_path(principal.canonical_project_root()),
    )
    .ok()
    .and_then(|prefs| prefs.registration)
    .is_some_and(|registration| {
        registration.session_id == principal.session_id()
            && live_pm_window_id(panes, principal.session_id()).is_some()
    });
    if !registered {
        return Err("caller_not_registered_pm");
    }
    if !project_available || !principal.authorizes_project_root(root) {
        return Err("project_not_open");
    }
    match crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(root)) {
        Ok(prefs) if prefs.enabled => Ok(()),
        Ok(_) => Err("monitor_disabled"),
        Err(error) => {
            tracing::warn!(%error, "owner Monitor scan permission read failed");
            Err("monitor_state_unavailable")
        }
    }
}

fn issue_monitor_scan_now_events(
    root: &Path,
    panes: &[PaneProjection],
    project_available: bool,
    client_id: String,
    principal: &AgentSessionPrincipal,
    expected_scope: &str,
) -> Vec<OutboundEvent> {
    let result =
        issue_monitor_scan_permission(root, panes, project_available, principal, expected_scope)
            .map_err(str::to_string)
            .and_then(|_| {
                crate::daemon_publisher::publish_issue_monitor_control(
                    root,
                    crate::runtime_daemon_events::issue_monitor_payload(
                        "control",
                        serde_json::json!({"scan_now_if_idle":{}}),
                        std::process::id(),
                    ),
                )
                .map_err(|error| {
                    if error.to_string()
                == crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_SCAN_ALREADY_IN_FLIGHT_ERROR
            {
                "scan_already_in_flight".to_string()
            } else {
                tracing::warn!(%error, "owner Monitor scan admission failed");
                "monitor_state_unavailable".to_string()
            }
                })
            });
    vec![OutboundEvent::reply(
        client_id,
        BackendEvent::IssueMonitorScanRequestResult {
            accepted: result.is_ok(),
            reason: result.err(),
        },
    )]
}

pub fn apply_dispatch_outcome(
    clients: &crate::project_transport::ClientHub,
    client_id: &str,
    outcome: super::agent_frontend::AgentFrontendDispatchOutcome,
) {
    use super::agent_frontend::AgentFrontendDispatchOutcome;
    match outcome {
        AgentFrontendDispatchOutcome::Dispatched(events) => clients.dispatch(events),
        AgentFrontendDispatchOutcome::StaleCapability => clients.unregister_with_close_frame(
            client_id,
            Some(crate::project_transport::AGENT_STALE_BINDING_CLOSE),
        ),
        AgentFrontendDispatchOutcome::ExecutionAuthorityUnavailable => clients
            .unregister_with_close_frame(
                client_id,
                Some(crate::project_transport::AGENT_AUTHORITY_UNAVAILABLE_CLOSE),
            ),
    }
}

pub(crate) type OwnerWindowRegistry = Arc<Mutex<crate::WindowRegistry>>;

pub(crate) fn load_owner_registry(root: &Path) -> Result<OwnerWindowRegistry, String> {
    crate::persistence::load_restored_workspace_state(root)
        .map(crate::WindowRegistry::from_persisted)
        .map(|registry| Arc::new(Mutex::new(registry)))
        .map_err(|error| format!("owner window registry unavailable: {error}"))
}

/// A typed lifecycle worker publishes identity; GUI layout cannot call this.
/// A failed durable write leaves the exact candidate in memory for cleanup,
/// rather than admitting another Session into an uncertain on-disk slot.
pub(crate) fn publish_owned_process_window(
    registry: &OwnerWindowRegistry,
    root: &Path,
    canonical_id: &str,
    expected: Option<&crate::persistence::PersistedWindowState>,
    mut window: crate::persistence::PersistedWindowState,
    current: &std::sync::atomic::AtomicBool,
) -> Result<(), OwnedProcessPublishFailure> {
    let untouched = |detail: String| OwnedProcessPublishFailure {
        detail,
        record_changed: false,
    };
    let raw = raw_owner_window_id(root, canonical_id).map_err(untouched)?;
    if window.id != raw {
        return Err(untouched(
            "owner registry row does not match canonical pane".into(),
        ));
    }
    let mut registry = registry
        .lock()
        .map_err(|error| untouched(error.to_string()))?;
    if !current.load(std::sync::atomic::Ordering::Acquire) {
        return Err(untouched(
            "pane was closed before registry publication".into(),
        ));
    }
    let current = registry.window(raw);
    let identity = |row: &crate::persistence::PersistedWindowState| {
        (
            row.id.clone(),
            row.preset,
            row.session_id.clone(),
            row.agent_id.clone(),
        )
    };
    if current.map(identity) != expected.map(identity) {
        return Err(untouched(
            "owner window record changed before lifecycle publication".into(),
        ));
    }
    if let Some(current) = current {
        window.placement = current.placement.clone();
        window.surface_owner = current.surface_owner;
        window.z_index = current.z_index;
    }
    let mut state = registry.persisted().clone();
    state.windows.retain(|row| row.id != raw);
    state.windows.push(window);
    *registry = crate::WindowRegistry::from_persisted(state);
    crate::persistence::save_workspace_state_durable(
        &crate::persistence::workspace_state_path(root),
        &registry.persistable_state(),
    )
    .map_err(|error| OwnedProcessPublishFailure {
        detail: error.to_string(),
        record_changed: true,
    })
}

#[derive(Clone)]
pub(crate) struct OwnedWindowRecordCleanup {
    pub registry: OwnerWindowRegistry,
    pub registry_path: PathBuf,
    pub root: PathBuf,
    pub project_key: String,
    pub canonical_id: String,
    pub expected_session_id: Option<String>,
}
impl OwnedWindowRecordCleanup {
    pub(crate) fn finish(&self) -> Result<(), String> {
        #[cfg(any(test, feature = "test-gh-guard"))]
        if let Some(session_id) = self.expected_session_id.as_deref() {
            super::agent_frontend::run_owner_agent_cleanup_test_hook(&self.root, session_id);
        }
        let raw = self
            .canonical_id
            .strip_prefix(&format!("{}::", self.project_key))
            .filter(|raw| !raw.is_empty())
            .ok_or("window is outside the captured fixed project scope")?;
        let mut registry = self.registry.lock().map_err(|error| error.to_string())?;
        if registry
            .window(raw)
            .is_some_and(|row| row.session_id != self.expected_session_id)
        {
            return Err("window Session changed before exact row cleanup".into());
        }
        registry.close_window(raw);
        // Persist even if an earlier failed sync already removed the in-memory
        // row. Success ACK is only possible after this durable barrier.
        crate::persistence::save_workspace_state_durable(
            &self.registry_path,
            &registry.persistable_state(),
        )
        .map_err(|error| error.to_string())
    }
}

pub(crate) struct StoppedWindowRetirement {
    pub sessions_dir: PathBuf,
    pub expected: Option<gwt_agent::Session>,
    pub cleanup: OwnedWindowRecordCleanup,
}
impl StoppedWindowRetirement {
    pub(crate) fn finish(&mut self) -> Result<(), String> {
        if let Some(expected) = &mut self.expected {
            if !close_stopped_session(&self.cleanup.root, &self.sessions_dir, expected)? {
                return Err("stopped Session changed or remains physically unconfirmed".into());
            }
            // A later registry fsync retry must use the already-committed exact
            // stopped snapshot rather than replaying an obsolete pre-write CAS.
            expected.restore_window_on_startup = false;
        }
        self.cleanup.finish()
    }
}

pub(crate) struct OwnedProcessPublishFailure {
    pub detail: String,
    pub record_changed: bool,
}

pub(crate) fn owned_process_window_state(
    root: &Path,
    canonical_id: &str,
    session: Option<&gwt_agent::Session>,
    active: Option<&super::launch_payload::ActiveAgentSession>,
    purpose_title: Option<String>,
    previous: Option<&crate::persistence::PersistedWindowState>,
) -> Result<crate::persistence::PersistedWindowState, String> {
    let raw = raw_owner_window_id(root, canonical_id)?;
    Ok(crate::persistence::PersistedWindowState {
        id: raw.into(),
        title: active.map_or_else(|| "Shell".into(), |active| active.display_name.clone()),
        purpose_title,
        dynamic_title: None,
        dynamic_title_detail: None,
        preset: if session.is_some() {
            crate::WindowPreset::Agent
        } else {
            crate::WindowPreset::Shell
        },
        surface_owner: previous.and_then(|row| row.surface_owner),
        z_index: previous.map_or(1, |row| row.z_index),
        status: crate::WindowProcessStatus::Running,
        placement: previous.map_or(crate::persistence::WindowPlacement::Surface, |row| {
            row.placement.clone()
        }),
        persist: true,
        agent_id: active.map(|active| active.agent_id.clone()),
        agent_name: session.and_then(|session| session.agent_name.clone()),
        agent_color: None,
        worktree_form: crate::persistence::WindowWorktreeForm::Unknown,
        session_id: session.map(|session| session.id.clone()),
        linked_issue_number: session.and_then(|session| session.linked_issue_number),
        runtime_started_at_ms: None,
        is_pm: false,
    })
}

pub(crate) fn forget_owner_writer(
    writers: &PtyWriterRegistry,
    id: &str,
    current: &Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    let mut writers = writers.write().map_err(|error| error.to_string())?;
    if let Some(entry) = writers.get(id) {
        if !Arc::ptr_eq(&entry.current, current) {
            return Err("writer registry belongs to another pane incarnation".into());
        }
        entry
            .current
            .store(false, std::sync::atomic::Ordering::Release);
        entry.handle.revoke_input_generation();
        writers.remove(id);
    }
    Ok(())
}

pub fn window_is_agent_pane(window: &crate::PersistedWindowState) -> bool {
    window.agent_id.is_some()
        || matches!(
            window.preset,
            crate::WindowPreset::Agent | crate::WindowPreset::Claude | crate::WindowPreset::Codex
        )
}

pub fn collect_running_agents(
    windows: &[crate::PersistedWindowState],
) -> Vec<crate::RunningAgentSummary> {
    windows
        .iter()
        .filter(|window| {
            window_is_agent_pane(window) && window.status == WindowProcessStatus::Running
        })
        .map(|window| crate::RunningAgentSummary {
            display_name: window
                .dynamic_title
                .clone()
                .or_else(|| window.purpose_title.clone())
                .unwrap_or_else(|| window.title.clone()),
            branch: window.dynamic_title_detail.clone(),
        })
        .collect()
}

/// Public projection uses qualified identities for the row and sibling surface
/// references. The authoritative registry keeps its stable raw components.
pub fn qualify_window_projection(window: &mut crate::PersistedWindowState, tab_id: &str) {
    window.id = format!("{tab_id}::{}", window.id);
    match &mut window.placement {
        crate::WindowPlacement::AgentKanban { board_id, .. } => {
            *board_id = format!("{tab_id}::{board_id}")
        }
        crate::WindowPlacement::IssuePreview {
            issue_window_id, ..
        } => *issue_window_id = format!("{tab_id}::{issue_window_id}"),
        crate::WindowPlacement::Surface => {}
    }
}
