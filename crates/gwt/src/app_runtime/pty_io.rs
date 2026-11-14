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
pub(super) fn split_pane_submit(text: &str) -> (&str, Option<&str>) {
    for terminator in ["\r\n", "\r", "\n"] {
        if let Some(body) = text.strip_suffix(terminator) {
            return (body, Some(terminator));
        }
    }
    (text, None)
}

/// Result of a body-once, submit-until-verified delivery. Verification is an
/// injected semantic acknowledgement (for PM wakeups, the exact
/// UserPromptSubmit marker), not a PTY write acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VerifiedPaneSubmitOutcome {
    Verified { submit_attempts: usize },
    Unverified { submit_attempts: usize },
}

/// Drive a provider-aware submit without ever duplicating the prompt body.
///
/// The caller owns the execution context: production runs this on the
/// blocking pool and injects an authority-checked PTY writer plus a durable
/// semantic probe; tests inject deterministic closures. `settle` runs before
/// every submit byte, preserving the separate-write contract introduced by
/// PR #3512.
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
    if max_submit_attempts == 0 {
        return Ok(VerifiedPaneSubmitOutcome::Unverified { submit_attempts: 0 });
    }

    for submit_attempts in 1..=max_submit_attempts {
        if (body.is_empty() || submit_attempts > 1) && is_verified()? {
            return Ok(VerifiedPaneSubmitOutcome::Verified {
                submit_attempts: submit_attempts - 1,
            });
        }
        settle(PANE_SUBMIT_SETTLE);
        if (body.is_empty() || submit_attempts > 1) && is_verified()? {
            return Ok(VerifiedPaneSubmitOutcome::Verified {
                submit_attempts: submit_attempts - 1,
            });
        }
        write(submit.as_bytes())?;
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
                .map_err(|error| error.to_string())
                .and_then(|pane| {
                    pane.write_input(body.as_bytes())
                        .map_err(|error| error.to_string())
                })
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
        let write_result = {
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

            match lock_result {
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
            }
        };

        match write_result {
            Ok(()) => Vec::new(),
            Err(error) => {
                self.handle_runtime_status(id.to_string(), WindowProcessStatus::Error, Some(error))
            }
        }
    }

    pub(crate) fn register_pty_writer(&mut self, id: &str, pane: &Arc<Mutex<Pane>>) {
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
        let previous = self
            .pty_writers
            .read()
            .ok()
            .and_then(|guard| guard.get(id).cloned());
        // A first registration after process restart is also a generation
        // boundary. Any durable protected-delivery fence belongs to a PTY
        // handle from the previous process and cannot be resumed safely.
        let generation_changed = previous
            .as_ref()
            .is_none_or(|previous| !Arc::ptr_eq(previous, &pty));
        if generation_changed {
            if let Some(previous) = previous.as_ref() {
                previous.invalidate_input_generation();
            }
        }
        match self.pty_writers.write() {
            Ok(mut guard) => {
                guard.insert(id.to_string(), pty);
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
        if generation_changed {
            self.clear_pm_submit_state_for_pty_generation_change(id);
        }
    }

    pub(crate) fn deregister_pty_writer(&mut self, id: &str) {
        if let Some(previous) = self
            .pty_writers
            .read()
            .ok()
            .and_then(|guard| guard.get(id).cloned())
        {
            previous.invalidate_input_generation();
        }
        match self.pty_writers.write() {
            Ok(mut guard) => {
                guard.remove(id);
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
        self.clear_pm_submit_state_for_pty_generation_change(id);
    }

    pub(crate) fn stop_window_runtime(&mut self, window_id: &str) {
        self.stop_window_runtime_inner(window_id, true);
    }

    pub(crate) fn stop_window_runtime_without_session_projection(&mut self, window_id: &str) {
        self.stop_window_runtime_inner(window_id, false);
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
        // Terminate the child before any capability/registration teardown can
        // wait on an already-linearized PTY write. `kill` uses the independent
        // child/process-group handles and unblocks a full PTY input buffer;
        // the remaining authority cleanup can then complete deterministically.
        if let Some(runtime) = self.runtimes.get(window_id) {
            if let Ok(pane) = runtime.pane.lock() {
                let _ = pane.kill();
            }
        }
        if mark_session_stopped {
            self.mark_agent_session_stopped(window_id);
        }
        self.remove_window_state_tracking(window_id);
        self.deregister_pty_writer(window_id);
        let mut threads = RuntimeStopThreads {
            output_thread: None,
            status_thread: None,
        };
        if let Some(mut runtime) = self.runtimes.remove(window_id) {
            threads.output_thread = runtime.output_thread.take();
            threads.status_thread = runtime.status_thread.take();
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
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
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
                            data: chunk,
                        });
                    }
                    Err(error) => {
                        proxy.send(UserEvent::RuntimeStatus {
                            id: id.clone(),
                            status: WindowProcessStatus::Error,
                            detail: Some(error.to_string()),
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
                    proxy.send(UserEvent::RuntimeStatus { id, status, detail });
                }
                Err(error) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
                    });
                }
            }
        })
    }

    pub(crate) fn spawn_status_thread(&self, id: String, pane: Arc<Mutex<Pane>>) -> JoinHandle<()> {
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
                    proxy.send(UserEvent::RuntimeStatus { id, status, detail });
                    break;
                }
                Err(error) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
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

    /// SPEC-3431 FR-108c follow-up: a submit that was accepted by the PTY but
    /// not observed by the provider is retried with the submit byte only. The
    /// prompt body is never duplicated in the composer.
    #[test]
    fn unverified_submit_retries_only_the_carriage_return() {
        let writes = std::cell::RefCell::new(Vec::new());

        let outcome = drive_verified_pane_submit(
            "wake body\r",
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

        assert_eq!(
            *writes.borrow(),
            ["wake body", "\r", "\r"],
            "only the submit byte may be retried"
        );
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
            "wake body\r",
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

        assert_eq!(*writes.borrow(), ["wake body", "\r"]);
        assert_eq!(
            outcome,
            VerifiedPaneSubmitOutcome::Verified { submit_attempts: 1 }
        );
    }

    #[test]
    fn verification_unavailable_stops_before_an_extra_submit_byte() {
        let writes = std::cell::RefCell::new(Vec::new());

        let error = drive_verified_pane_submit(
            "wake body\r",
            2,
            |bytes| {
                writes.borrow_mut().push(bytes.to_vec());
                Ok(())
            },
            |_| {},
            || {
                if writes.borrow().len() >= 2 {
                    Err("ack store unavailable".to_string())
                } else {
                    Ok(false)
                }
            },
        )
        .expect_err("an unreadable acknowledgement store is not semantic absence");

        assert_eq!(*writes.borrow(), [b"wake body".to_vec(), b"\r".to_vec()]);
        assert!(error.contains("ack store unavailable"));
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
