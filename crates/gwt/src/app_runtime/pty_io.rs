//! Terminal/pane IO bridging + PTY runtime thread lifecycle split out of
//! `app_runtime/mod.rs` for SPEC-3064 Phase 1 (Pass 2).
//!
//! Owns:
//! - Client-facing pane input bridging
//!   ([`AppRuntime::pane_send_input_events`],
//!   [`AppRuntime::terminal_input_events`],
//!   [`AppRuntime::client_pane_snapshot_repair_events`])
//! - The PTY writer registry ([`AppRuntime::register_pty_writer`] /
//!   [`AppRuntime::deregister_pty_writer`])
//! - Runtime stop orchestration ([`AppRuntime::stop_window_runtime`],
//!   [`AppRuntime::stop_all_runtimes`], the `RuntimeStopThreads` join
//!   helpers) and the PTY output / status watcher threads
//!   ([`AppRuntime::spawn_output_thread`],
//!   [`AppRuntime::spawn_status_thread`])
//!
//! Behavior-preserving move: `WindowRuntime` / `RuntimeStopThreads` stay in
//! `mod.rs` and are reached via `super`.

use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine as _;

use super::{
    combined_window_id, AppRuntime, BackendEvent, ClientId, OutboundEvent, Pane, PaneStatus,
    Read as _, RuntimeStopThreads, UserEvent, WindowProcessStatus,
};

/// SPEC-3431 FR-108c: how long the TUI is given to render the injected body
/// before the submit byte arrives. Claude Code and Codex both fold a carriage
/// return that lands in the same PTY write as the text into the text itself —
/// the line is inserted on the prompt but never submitted — so the submit has
/// to be a write of its own, after the TUI has settled.
const PANE_SUBMIT_SETTLE: Duration = Duration::from_millis(400);

/// Split one pane payload into the body and its submit terminator. Input that
/// carries no terminator is not a submit and is returned whole, so raw
/// keystroke forwarding stays byte-exact.
fn split_pane_submit(text: &str) -> (&str, Option<&str>) {
    for terminator in ["\r\n", "\r", "\n"] {
        if let Some(body) = text.strip_suffix(terminator) {
            return (body, Some(terminator));
        }
    }
    (text, None)
}

/// Result of a body-once, submit-until-verified delivery. Verification is a
/// semantic target acknowledgement, never merely a successful PTY write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VerifiedPaneSubmitOutcome {
    Verified { submit_attempts: usize },
    Unverified { submit_attempts: usize },
}

/// Write the prompt body once, then retry only its submit terminator until the
/// injected semantic acknowledgement says the exact target turn started.
pub(super) fn drive_verified_pane_submit(
    text: &str,
    max_submit_attempts: usize,
    mut write: impl FnMut(&[u8]) -> Result<(), String>,
    mut settle: impl FnMut(Duration),
    mut is_verified: impl FnMut() -> Result<bool, String>,
) -> Result<VerifiedPaneSubmitOutcome, String> {
    let (body, submit) = split_pane_submit(text);
    if !body.is_empty() {
        write(body.as_bytes())?;
    }
    let Some(submit) = submit else {
        return Ok(VerifiedPaneSubmitOutcome::Verified { submit_attempts: 0 });
    };

    // Give the TUI composer time to consume the body before the first submit.
    settle(PANE_SUBMIT_SETTLE);
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
pub(super) fn write_pane_input_then_submit(
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
        reservation
            .write_input(body.as_bytes())
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

/// Complete the stop phase for every runtime before any join can block.
fn stop_all_before_joining<I, T>(
    ids: I,
    mut stop: impl FnMut(I::Item) -> T,
    mut join: impl FnMut(T),
) where
    I: IntoIterator,
{
    let stopped = ids.into_iter().map(&mut stop).collect::<Vec<_>>();
    for threads in stopped {
        join(threads);
    }
}

impl AppRuntime {
    /// SPEC-2359 W-17 (FR-396): re-send full snapshots for panes whose
    /// streamed output was dropped under client queue pressure, restoring
    /// display consistency for the affected client only.
    pub(crate) fn client_pane_snapshot_repair_events(
        &self,
        client_id: &str,
        pane_ids: &[String],
    ) -> Vec<OutboundEvent> {
        pane_ids
            .iter()
            .filter_map(|id| {
                let runtime = self.runtimes.get(id)?;
                let snapshot = runtime
                    .pane
                    .lock()
                    .map(|pane| pane.snapshot_bytes())
                    .unwrap_or_default();
                (!snapshot.is_empty()).then(|| {
                    OutboundEvent::reply(
                        client_id,
                        BackendEvent::TerminalSnapshot {
                            id: id.clone(),
                            data_base64: base64::engine::general_purpose::STANDARD.encode(snapshot),
                        },
                    )
                })
            })
            .collect()
    }

    /// SPEC-3050 FR-001/FR-002: inject one line of input into the pane bound
    /// to `session_id`. The event carries a session id instead of a window id,
    /// so a caller can only ever reach the pane of the session it presents;
    /// resolution + the live-runtime check both reply with an explicit
    /// `pane_send_result` (FR-005: no silent drop, unlike `terminal_input`).
    pub(crate) fn pane_send_input_events(
        &mut self,
        client_id: ClientId,
        session_id: &str,
        text: &str,
    ) -> Vec<OutboundEvent> {
        let target = self.tabs.iter().find_map(|tab| {
            tab.workspace
                .persisted()
                .windows
                .iter()
                .find(|window| window.session_id.as_deref() == Some(session_id))
                .map(|window| combined_window_id(&tab.id, &window.id))
        });
        let Some(window_id) = target else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PaneSendResult {
                    ok: false,
                    window_id: None,
                    error: Some(format!("no pane bound to session {session_id}")),
                },
            )];
        };

        self.pane_send_input_to_window_events(client_id, &window_id, text)
    }

    /// Inject input into one already-authorized pane identity. Capability
    /// callers resolve this exact combined window id inside their authenticated
    /// project before reaching the PTY; this helper never performs a
    /// process-global Session lookup.
    pub(crate) fn pane_send_input_to_window_events(
        &mut self,
        client_id: ClientId,
        window_id: &str,
        text: &str,
    ) -> Vec<OutboundEvent> {
        let write_result = match self.runtimes.get(window_id) {
            None => Err(format!("no live runtime for pane {window_id}")),
            Some(runtime) => write_pane_input_then_submit(&runtime.pane, text),
        };

        match write_result {
            Ok(()) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PaneSendResult {
                    ok: true,
                    window_id: Some(window_id.to_string()),
                    error: None,
                },
            )],
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PaneSendResult {
                    ok: false,
                    window_id: Some(window_id.to_string()),
                    error: Some(error),
                },
            )],
        }
    }

    pub(crate) fn terminal_input_events(&mut self, id: &str, data: &str) -> Vec<OutboundEvent> {
        let (incarnation, write_result) = {
            let Some(runtime) = self.runtimes.get(id) else {
                tracing::debug!(
                    target: "gwt_input_trace",
                    stage = "event_loop_runtime_missing",
                    window_id = %id,
                    outcome = "runtime_missing",
                    "terminal_input dropped: no runtime for window"
                );
                return Vec::new();
            };

            let lock_started = Instant::now();
            let lock_result = runtime.pane.lock().map_err(|error| error.to_string());
            let lock_wait_us = lock_started.elapsed().as_micros() as u64;

            let write_result = match lock_result {
                Ok(pane) => {
                    let write_started = Instant::now();
                    let result = pane
                        .write_input(data.as_bytes())
                        .map_err(|error| error.to_string());
                    tracing::debug!(
                        target: "gwt_input_trace",
                        stage = "pty_write",
                        window_id = %id,
                        lock_wait_us,
                        write_us = write_started.elapsed().as_micros() as u64,
                        ok = result.is_ok(),
                        "terminal_input forwarded to PTY writer"
                    );
                    result
                }
                Err(error) => {
                    tracing::debug!(
                        target: "gwt_input_trace",
                        stage = "pane_lock_failed",
                        window_id = %id,
                        lock_wait_us,
                        outcome = "lock_failed",
                        "terminal_input dropped: pane mutex poisoned"
                    );
                    Err(error)
                }
            };
            (runtime.incarnation, write_result)
        };

        match write_result {
            Ok(()) => Vec::new(),
            Err(error) => self.handle_runtime_status_event(
                id.to_string(),
                incarnation,
                WindowProcessStatus::Error,
                Some(error),
                false,
            ),
        }
    }

    pub(crate) fn register_pty_writer(&self, id: &str, pane: &Arc<Mutex<Pane>>) {
        let Ok(pane_guard) = pane.lock() else {
            tracing::warn!(
                target: "gwt_input_trace",
                stage = "registry_lock_poisoned",
                window_id = %id,
                "failed to register PTY writer: pane mutex poisoned"
            );
            return;
        };
        let pty = pane_guard.shared_pty();
        drop(pane_guard);
        match self.pty_writers.write() {
            Ok(mut guard) => {
                let previous = guard.insert(id.to_string(), Arc::clone(&pty));
                drop(guard);
                if let Some(previous) = previous.filter(|previous| !Arc::ptr_eq(previous, &pty)) {
                    previous.invalidate_input_generation();
                }
            }
            Err(_error) => {
                tracing::warn!(
                    target: "gwt_input_trace",
                    stage = "registry_write_poisoned",
                    window_id = %id,
                    outcome = "registry_lock_failed",
                    "failed to register PTY writer: registry poisoned"
                );
            }
        }
    }

    pub(crate) fn deregister_pty_writer(&self, id: &str) {
        match self.pty_writers.write() {
            Ok(mut guard) => {
                let previous = guard.remove(id);
                drop(guard);
                if let Some(previous) = previous {
                    previous.invalidate_input_generation();
                }
            }
            Err(_error) => {
                tracing::warn!(
                    target: "gwt_input_trace",
                    stage = "registry_deregister_poisoned",
                    window_id = %id,
                    outcome = "registry_lock_failed",
                    "failed to deregister PTY writer: registry poisoned"
                );
            }
        }
    }

    pub(crate) fn stop_window_runtime(&mut self, window_id: &str) {
        let exact_holder = self
            .active_agent_sessions
            .get(window_id)
            .and_then(|active| {
                let runtime_incarnation = self.runtimes.get(window_id)?.incarnation;
                let session = gwt_agent::Session::load(
                    &self
                        .sessions_dir
                        .join(format!("{}.toml", active.session_id)),
                )
                .ok()?;
                let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
                    .ok()
                    .flatten()?;
                Some((identity, runtime_incarnation))
            });
        if let Some((identity, runtime_incarnation)) = exact_holder {
            if self
                .stop_exact_manual_holder_runtime(window_id, runtime_incarnation, &identity, false)
                .is_ok()
            {
                return;
            }
            // Exact authority could not be fenced. Still stop the local
            // process for the user's close request, but retain Running
            // authority bytes so another Host cannot mistake the failure for
            // terminal successor permission.
            let threads = self.start_window_runtime_stop(window_id, false);
            Self::join_runtime_stop_threads(threads);
            self.mark_agent_session_stopped_with_persistence(window_id, false);
            return;
        }
        self.stop_window_runtime_inner(window_id, true);
    }

    pub(crate) fn stop_window_runtime_without_session_projection(&mut self, window_id: &str) {
        self.stop_window_runtime_inner(window_id, false);
    }

    /// Stop one exact locally-owned producing runtime. The runtime
    /// incarnation and Session execution identity are rechecked before the
    /// kill, and durable terminal evidence is written only after the child is
    /// observed exited. Any failure leaves Session/sidecar authority bytes
    /// untouched.
    pub(crate) fn stop_exact_manual_holder_runtime(
        &mut self,
        window_id: &str,
        expected_incarnation: u64,
        expected_session: &gwt_agent::SessionExecutionIdentity,
        retain_successor_handoff: bool,
    ) -> Result<(), String> {
        let active = self
            .active_agent_sessions
            .get(window_id)
            .filter(|active| active.session_id == expected_session.session_id)
            .cloned()
            .ok_or_else(|| "The exact holder Session is no longer local".to_string())?;
        let runtime = self
            .runtimes
            .get(window_id)
            .filter(|runtime| runtime.incarnation == expected_incarnation)
            .ok_or_else(|| "The exact holder runtime incarnation changed".to_string())?;
        let pane = runtime.pane.clone();
        let capability = self
            .agent_capability_issuer
            .clone()
            .zip(self.agent_capability_tokens.get(window_id).cloned());
        if retain_successor_handoff && capability.is_none() {
            return Err("The exact holder capability is unavailable".to_string());
        }
        let sessions_dir = self.sessions_dir.clone();
        let proof = gwt_agent::ManualLaunchRuntimeProof {
            host_pid: std::process::id(),
            runtime_incarnation: expected_incarnation,
        };
        let stopped = gwt::cli::execution_state::with_exact_active_manual_runtime_lease(
            &sessions_dir,
            expected_session,
            proof,
            retain_successor_handoff,
            || {
                let reservation = capability
                    .as_ref()
                    .map(|(issuer, token)| {
                        issuer
                            .begin_manual_execution_handoff(
                                token,
                                &expected_session.execution_binding,
                            )
                            .map(|reservation| (issuer, reservation))
                    })
                    .transpose()
                    .map_err(std::io::Error::other)?;
                let stop_result = (|| {
                    pane.lock()
                        .map_err(|error| std::io::Error::other(error.to_string()))?
                        .kill()
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                    let deadline = Instant::now() + Duration::from_secs(2);
                    loop {
                        if pane
                            .lock()
                            .map_err(|error| std::io::Error::other(error.to_string()))?
                            .process_has_exited()
                            .map_err(|error| std::io::Error::other(error.to_string()))?
                        {
                            break;
                        }
                        if Instant::now() >= deadline {
                            return Err(std::io::Error::other(
                                "The exact holder process did not exit after termination",
                            ));
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    if !gwt_agent::persist_session_terminal_status_if_execution_identity_matches_under_lease(
                        &sessions_dir,
                        expected_session,
                        expected_incarnation,
                        gwt_agent::AgentStatus::Stopped,
                    )? {
                        return Err(std::io::Error::other(
                            "The holder Session changed before terminal proof persistence",
                        ));
                    }
                    Ok(())
                })();
                match stop_result {
                    Ok(()) => {
                        if let Some((issuer, reservation)) = reservation.as_ref() {
                            if !issuer.commit_manual_execution_handoff(reservation) {
                                return Err(std::io::Error::other(
                                    "The exact holder handoff reservation was lost",
                                ));
                            }
                            if !retain_successor_handoff
                                && !issuer.release_manual_execution_handoff(reservation)
                            {
                                return Err(std::io::Error::other(
                                    "The completed holder capability fence could not be released",
                                ));
                            }
                        }
                        Ok(())
                    }
                    Err(error) => {
                        if let Some((issuer, reservation)) = reservation.as_ref() {
                            if !issuer.rollback_manual_execution_handoff(reservation) {
                                return Err(std::io::Error::other(format!(
                                    "{error}; exact holder capability rollback failed"
                                )));
                            }
                        }
                        Err(error)
                    }
                }
            },
        )
        .map_err(|error| error.to_string())?;
        if stopped.is_none() {
            return Err(
                "The exact durable holder authority changed before termination".to_string(),
            );
        }
        if self
            .runtimes
            .get(window_id)
            .is_none_or(|runtime| runtime.incarnation != expected_incarnation)
            || self
                .active_agent_sessions
                .get(window_id)
                .is_none_or(|current| current.session_id != active.session_id)
        {
            return Err("The holder runtime changed while termination was confirmed".to_string());
        }
        let threads = self.start_window_runtime_stop(window_id, false);
        Self::join_runtime_stop_threads(threads);
        self.mark_agent_session_stopped_with_persistence(window_id, false);
        Ok(())
    }

    fn stop_window_runtime_inner(&mut self, window_id: &str, mark_session_stopped: bool) {
        let threads = self.start_window_runtime_stop(window_id, mark_session_stopped);
        Self::join_runtime_stop_threads(threads);
    }

    fn start_window_runtime_stop(
        &mut self,
        window_id: &str,
        mark_session_stopped: bool,
    ) -> RuntimeStopThreads {
        let exact_terminal = if mark_session_stopped {
            self.active_agent_sessions
                .get(window_id)
                .and_then(|active| {
                    let runtime = self.runtimes.get(window_id)?;
                    let session = gwt_agent::Session::load(
                        &self
                            .sessions_dir
                            .join(format!("{}.toml", active.session_id)),
                    )
                    .ok()?;
                    let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
                        .ok()
                        .flatten()?;
                    Some((identity, runtime.incarnation))
                })
        } else {
            None
        };
        self.remove_window_state_tracking(window_id);
        self.deregister_pty_writer(window_id);
        let mut threads = RuntimeStopThreads {
            output_thread: None,
            status_thread: None,
        };
        let mut exact_terminal_persisted = false;
        if let Some(mut runtime) = self.runtimes.remove(window_id) {
            if let Ok(pane) = runtime.pane.lock() {
                let _ = pane.kill();
                if let Some((identity, incarnation)) = exact_terminal.as_ref() {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    let mut exited = false;
                    while Instant::now() < deadline {
                        if pane.process_has_exited().unwrap_or(false) {
                            exited = true;
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    if exited {
                        exact_terminal_persisted = gwt_agent::persist_session_terminal_status_if_execution_identity_matches(
                            &self.sessions_dir,
                            identity,
                            *incarnation,
                            gwt_agent::AgentStatus::Stopped,
                        ).unwrap_or(false);
                    }
                }
            }
            threads.output_thread = runtime.output_thread.take();
            threads.status_thread = runtime.status_thread.take();
        }
        if mark_session_stopped {
            self.mark_agent_session_stopped_with_persistence(
                window_id,
                exact_terminal.is_none() && !exact_terminal_persisted,
            );
        }
        self.window_details.remove(window_id);
        threads
    }

    fn join_runtime_stop_threads(mut threads: RuntimeStopThreads) {
        if let Some(handle) = threads.output_thread.take() {
            // PTY and its process group were already terminated by
            // `pane.kill()`, so the reader should see EOF quickly. Cap
            // the wait anyway so shutdown never stalls the event loop
            // if a stuck syscall keeps the reader in `read`. If the
            // timeout elapses the reader thread is detached; its Arc
            // clone of the Pane will still be released when the thread
            // does finally observe EOF.
            let (tx, rx) = std_mpsc::channel();
            thread::spawn(move || {
                let _ = handle.join();
                let _ = tx.send(());
            });
            let _ = rx.recv_timeout(Duration::from_millis(500));
        }
        if let Some(handle) = threads.status_thread.take() {
            let (tx, rx) = std_mpsc::channel();
            thread::spawn(move || {
                let _ = handle.join();
                let _ = tx.send(());
            });
            let _ = rx.recv_timeout(Duration::from_millis(500));
        }
    }

    /// Stop every active window runtime. Called from the application shutdown
    /// paths so no PTY / agent process outlives the GUI.
    pub(crate) fn stop_all_runtimes(&mut self) {
        let ids: Vec<String> = self.runtimes.keys().cloned().collect();
        self.stop_runtimes_in_shutdown_order(ids);
    }

    pub(super) fn stop_runtimes_in_shutdown_order(&mut self, ids: Vec<String>) {
        stop_all_before_joining(
            ids,
            |id| self.start_window_runtime_stop(&id, false),
            Self::join_runtime_stop_threads,
        );
    }

    pub(crate) fn spawn_output_thread(
        &self,
        id: String,
        incarnation: u64,
        pane: Arc<Mutex<Pane>>,
        _console_kind: Option<gwt_core::process_console::ProcessKind>,
    ) -> JoinHandle<()> {
        // SPEC-2809 (revised) — the Console window is the gwt-side
        // equivalent of VS Code's Output panel. It surfaces what gwt
        // itself spawns in the background (gh / git / docker / agent
        // bootstrap stages / Python index runner) per kind. The agent
        // tab is for the **Launch Wizard pipeline** that culminates in
        // the PTY spawn — not the agent's own runtime stdout. That
        // runtime stdout already lives in the workspace terminal pane
        // (xterm.js) and would only duplicate noise here. `_console_kind`
        // is retained on the API for forward compatibility with future
        // kind-aware hooks (e.g. recording the PTY exit code as a
        // summary at thread end).
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let reader = match pane
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|pane| pane.reader().map_err(|error| error.to_string()))
            {
                Ok(reader) => reader,
                Err(error) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        incarnation,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
                        exit_confirmed: false,
                    });
                    return;
                }
            };

            let mut reader = reader;
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        let chunk = buffer[..read].to_vec();
                        let lock_started = Instant::now();
                        if let Ok(mut pane) = pane.lock() {
                            let lock_wait_us = lock_started.elapsed().as_micros() as u64;
                            let parse_started = Instant::now();
                            pane.process_bytes(&chunk);
                            let parse_us = parse_started.elapsed().as_micros() as u64;
                            // Log only when the contention window is large enough
                            // to plausibly starve a concurrent `write_input`. The
                            // threshold keeps the log volume bounded during
                            // normal output bursts while still surfacing the
                            // lock-hold windows that matter for drop triage.
                            if lock_wait_us > 500 || parse_us > 500 {
                                tracing::debug!(
                                    target: "gwt_input_trace",
                                    stage = "reader_pane_lock",
                                    window_id = %id,
                                    lock_wait_us,
                                    parse_us,
                                    "reader thread held pane mutex (output parsing)"
                                );
                            }
                        }
                        proxy.send(UserEvent::RuntimeOutput {
                            id: id.clone(),
                            incarnation,
                            data: chunk,
                        });
                    }
                    Err(error) => {
                        proxy.send(UserEvent::RuntimeStatus {
                            id: id.clone(),
                            incarnation,
                            status: WindowProcessStatus::Error,
                            detail: Some(error.to_string()),
                            exit_confirmed: false,
                        });
                        return;
                    }
                }
            }

            let status = pane
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|mut pane| {
                    pane.check_status()
                        .cloned()
                        .map_err(|error| error.to_string())
                });

            match status {
                Ok(status) => {
                    let (status, detail) = Self::runtime_status_from_pane_status(&status);
                    let exit_confirmed = pane
                        .lock()
                        .ok()
                        .and_then(|pane| pane.process_has_exited().ok())
                        .unwrap_or(false);
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        incarnation,
                        status,
                        detail,
                        exit_confirmed,
                    });
                }
                Err(error) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        incarnation,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
                        exit_confirmed: false,
                    });
                }
            }
        })
    }

    pub(crate) fn spawn_status_thread(
        &self,
        id: String,
        incarnation: u64,
        pane: Arc<Mutex<Pane>>,
    ) -> JoinHandle<()> {
        let proxy = self.proxy.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(100));
            let status = pane
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|mut pane| {
                    pane.check_status()
                        .cloned()
                        .map_err(|error| error.to_string())
                });

            match status {
                Ok(PaneStatus::Running) => continue,
                Ok(status) => {
                    if matches!(status, PaneStatus::Completed(_)) {
                        if let Ok(pane) = pane.lock() {
                            let _ = pane.kill();
                        }
                    }
                    let (status, detail) = Self::runtime_status_from_pane_status(&status);
                    let exit_confirmed = pane
                        .lock()
                        .ok()
                        .and_then(|pane| pane.process_has_exited().ok())
                        .unwrap_or(false);
                    proxy.send(UserEvent::RuntimeStatus {
                        id: id.clone(),
                        incarnation,
                        status,
                        detail,
                        exit_confirmed,
                    });
                    if exit_confirmed {
                        break;
                    }
                }
                Err(error) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        incarnation,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
                        exit_confirmed: false,
                    });
                    break;
                }
            }
        })
    }

    fn runtime_status_from_pane_status(
        status: &PaneStatus,
    ) -> (WindowProcessStatus, Option<String>) {
        match status {
            PaneStatus::Running => (WindowProcessStatus::Running, None),
            PaneStatus::Completed(0) => (
                gwt::window_state::window_state_from_pane_status(status),
                Some("Process exited".to_string()),
            ),
            PaneStatus::Completed(code) => (
                gwt::window_state::window_state_from_pane_status(status),
                Some(format!("Process exited with status {code}")),
            ),
            PaneStatus::Error(message) => (
                gwt::window_state::window_state_from_pane_status(status),
                Some(message.clone()),
            ),
        }
    }
}

#[cfg(test)]
mod submit_split_tests {
    use super::{drive_verified_pane_submit, split_pane_submit, VerifiedPaneSubmitOutcome};

    /// SPEC-3431 FR-108c: the body and the submit byte must be separable, so
    /// the writer can put the carriage return in its own PTY write. A single
    /// write of `body + CR` is what the TUIs mis-read as a literal newline.
    #[test]
    fn carriage_return_is_separated_from_the_body() {
        assert_eq!(split_pane_submit("digest.\r"), ("digest.", Some("\r")));
        assert_eq!(split_pane_submit("digest.\n"), ("digest.", Some("\n")));
    }

    /// A CRLF terminator is one submit, not a body that ends in CR: stripping
    /// only the LF would leave a stray carriage return glued to the text.
    #[test]
    fn crlf_is_treated_as_a_single_submit_terminator() {
        assert_eq!(split_pane_submit("digest.\r\n"), ("digest.", Some("\r\n")));
    }

    /// Input without a terminator is not a submit — `terminal_input` forwards
    /// raw keystrokes through the same writer and must stay byte-exact.
    #[test]
    fn payload_without_a_terminator_is_written_whole() {
        assert_eq!(split_pane_submit("partial"), ("partial", None));
        assert_eq!(split_pane_submit(""), ("", None));
    }

    /// A bare terminator (the submit-only follow-up a caller may send) keeps an
    /// empty body so the writer skips the first write entirely.
    #[test]
    fn bare_terminator_has_an_empty_body() {
        assert_eq!(split_pane_submit("\r"), ("", Some("\r")));
    }

    #[test]
    fn unverified_submit_retries_only_the_carriage_return() {
        let writes = std::cell::RefCell::new(Vec::new());

        let outcome = drive_verified_pane_submit(
            "protected body\r",
            2,
            |bytes| {
                writes
                    .borrow_mut()
                    .push(String::from_utf8(bytes.to_vec()).expect("utf8 input"));
                Ok(())
            },
            |_| {},
            || Ok(writes.borrow().len() == 3),
        )
        .expect("second submit is verified");

        assert_eq!(*writes.borrow(), ["protected body", "\r", "\r"]);
        assert_eq!(
            outcome,
            VerifiedPaneSubmitOutcome::Verified { submit_attempts: 2 }
        );
    }

    #[test]
    fn delayed_ack_before_retry_suppresses_the_extra_carriage_return() {
        let writes = std::cell::RefCell::new(Vec::new());
        let acknowledged = std::cell::Cell::new(false);
        let settles = std::cell::Cell::new(0_usize);

        let outcome = drive_verified_pane_submit(
            "protected body\r",
            2,
            |bytes| {
                writes
                    .borrow_mut()
                    .push(String::from_utf8(bytes.to_vec()).expect("utf8 input"));
                Ok(())
            },
            |_| {
                let next = settles.get() + 1;
                settles.set(next);
                if next == 2 {
                    acknowledged.set(true);
                }
            },
            || Ok(acknowledged.get()),
        )
        .expect("delayed acknowledgement settles the original submit");

        assert_eq!(*writes.borrow(), ["protected body", "\r"]);
        assert_eq!(
            outcome,
            VerifiedPaneSubmitOutcome::Verified { submit_attempts: 1 }
        );
    }

    #[test]
    fn final_submit_gets_an_ack_grace_period() {
        let writes = std::cell::RefCell::new(Vec::new());
        let acknowledged = std::cell::Cell::new(false);
        let settles = std::cell::Cell::new(0_usize);

        let outcome = drive_verified_pane_submit(
            "protected body\r",
            1,
            |bytes| {
                writes
                    .borrow_mut()
                    .push(String::from_utf8(bytes.to_vec()).expect("utf8 input"));
                Ok(())
            },
            |_| {
                let next = settles.get() + 1;
                settles.set(next);
                if next == 2 {
                    acknowledged.set(true);
                }
            },
            || Ok(acknowledged.get()),
        )
        .expect("final submit acknowledgement is observed during its grace period");

        assert_eq!(*writes.borrow(), ["protected body", "\r"]);
        assert_eq!(
            outcome,
            VerifiedPaneSubmitOutcome::Verified { submit_attempts: 1 }
        );
    }
}

#[cfg(test)]
mod shutdown_order_tests {
    use std::cell::RefCell;

    use super::stop_all_before_joining;

    #[test]
    fn stops_every_runtime_before_joining_any_runtime() {
        let events = RefCell::new(Vec::new());

        stop_all_before_joining(
            ["a", "b"],
            |id| {
                events.borrow_mut().push(format!("stop:{id}"));
                id
            },
            |id| events.borrow_mut().push(format!("join:{id}")),
        );

        assert_eq!(
            events.into_inner(),
            ["stop:a", "stop:b", "join:a", "join:b"]
        );
    }
}
