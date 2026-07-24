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

use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc as std_mpsc, Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine as _;

use super::{
    combined_window_id, AppRuntime, BackendEvent, ClientId, OutboundEvent, Pane, PaneStatus,
    Read as _, RuntimeStopThreads, UserEvent, WindowProcessStatus,
};

static PTY_OUTPUT_TRACE_READER_ID: AtomicU64 = AtomicU64::new(1);

#[inline]
fn next_pty_output_trace_reader_id() -> u64 {
    loop {
        let reader_id = PTY_OUTPUT_TRACE_READER_ID.fetch_add(1, Ordering::Relaxed);
        if reader_id != 0 {
            return reader_id;
        }
    }
}

#[derive(Debug)]
pub(super) struct PtyOutputTraceReader {
    reader_id: u64,
    seq: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PtyOutputTraceRead {
    reader_id: u64,
    seq: u64,
}

impl PtyOutputTraceReader {
    pub(super) fn new() -> Self {
        Self {
            reader_id: next_pty_output_trace_reader_id(),
            seq: 0,
        }
    }

    #[inline]
    pub(super) fn record_successful_read(&mut self, id: &str) -> Option<PtyOutputTraceRead> {
        if !tracing::enabled!(target: "gwt_input_trace", tracing::Level::DEBUG) {
            return None;
        }

        self.seq = self.seq.wrapping_add(1);
        if self.seq == 0 {
            self.reader_id = next_pty_output_trace_reader_id();
            self.seq = 1;
        }
        let read = PtyOutputTraceRead {
            reader_id: self.reader_id,
            seq: self.seq,
        };
        trace_pty_output_arrival(id, read.reader_id, read.seq);
        Some(read)
    }

    #[cfg(test)]
    pub(super) fn set_seq_for_test(&mut self, seq: u64) {
        self.seq = seq;
    }
}

impl PtyOutputTraceRead {
    #[inline]
    pub(super) fn trace_slow_reader_pane_lock(self, id: &str, lock_wait_us: u64, parse_us: u64) {
        trace_slow_reader_pane_lock(id, self.reader_id, self.seq, lock_wait_us, parse_us);
    }
}

#[inline]
fn trace_pty_output_arrival(id: &str, reader_id: u64, seq: u64) {
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "pty_output_arrival",
        window_id = %id,
        reader_id,
        seq,
        "PTY output arrived at reader"
    );
}

#[inline]
fn trace_slow_reader_pane_lock(
    id: &str,
    reader_id: u64,
    seq: u64,
    lock_wait_us: u64,
    parse_us: u64,
) {
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "reader_pane_lock",
        window_id = %id,
        reader_id,
        seq,
        lock_wait_us,
        parse_us,
        outcome = "slow_parse",
        "reader thread held pane mutex (output parsing)"
    );
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

        let write_result = match self.runtimes.get(&window_id) {
            None => Err(format!("no live runtime for pane {window_id}")),
            Some(runtime) => runtime
                .pane
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|pane| {
                    pane.write_input(text.as_bytes())
                        .map_err(|error| error.to_string())
                }),
        };

        match write_result {
            Ok(()) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PaneSendResult {
                    ok: true,
                    window_id: Some(window_id),
                    error: None,
                },
            )],
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PaneSendResult {
                    ok: false,
                    window_id: Some(window_id),
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
                    let outcome = if result.is_ok() {
                        "success"
                    } else {
                        "write_failed"
                    };
                    tracing::debug!(
                        target: "gwt_input_trace",
                        stage = "pty_write",
                        window_id = %id,
                        lock_wait_us,
                        write_us = write_started.elapsed().as_micros() as u64,
                        outcome,
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

    pub(crate) fn register_pty_writer(&self, id: &str, pane: &Arc<Mutex<Pane>>) {
        let Ok(pane_guard) = pane.lock() else {
            tracing::warn!(
                target: "gwt_input_trace",
                stage = "registry_lock_poisoned",
                window_id = %id,
                outcome = "pane_lock_failed",
                "failed to register PTY writer: pane mutex poisoned"
            );
            return;
        };
        let pty = pane_guard.shared_pty();
        drop(pane_guard);
        match self.pty_writers.write() {
            Ok(mut guard) => {
                guard.insert(id.to_string(), pty);
            }
            Err(_) => {
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
                guard.remove(id);
            }
            Err(_) => {
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
        self.stop_window_runtime_inner(window_id, true);
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
            if let Ok(pane) = runtime.pane.lock() {
                let _ = pane.kill();
            }
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
            let mut output_trace_reader = PtyOutputTraceReader::new();
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        let output_trace_read = output_trace_reader.record_successful_read(&id);
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
                                if let Some(output_trace_read) = output_trace_read {
                                    output_trace_read.trace_slow_reader_pane_lock(
                                        &id,
                                        lock_wait_us,
                                        parse_us,
                                    );
                                }
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
