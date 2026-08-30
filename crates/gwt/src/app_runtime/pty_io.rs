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

use std::sync::{mpsc as std_mpsc, Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine as _;

use super::{
    combined_window_id, AgentCapabilityIssuer, AppRuntime, BackendEvent, ClientId, OutboundEvent,
    Pane, PaneStatus, Read as _, RuntimeStopThreads, UserEvent, WindowCloseMonitorResult,
    WindowProcessStatus,
};

fn window_lifecycle_generation_is_current(
    generations: &Arc<Mutex<std::collections::HashMap<String, u64>>>,
    window_id: &str,
    expected: Option<u64>,
) -> bool {
    let Some(expected) = expected else {
        return false;
    };
    generations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(window_id)
        .is_some_and(|current| *current == expected)
}

fn settle_window_lifecycle_generation(
    generations: &Arc<Mutex<std::collections::HashMap<String, u64>>>,
    window_id: &str,
    expected: Option<u64>,
) {
    let Some(expected) = expected else {
        return;
    };
    let mut generations = generations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if generations.get(window_id).copied() == Some(expected) {
        generations.remove(window_id);
    }
}

struct CloseHandoffReservation {
    issuer: AgentCapabilityIssuer,
    reservation: crate::embedded_server::ManualExecutionHandoffReservation,
    state: CloseHandoffState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseHandoffState {
    Pending,
    Committed,
    Settled,
}

impl CloseHandoffReservation {
    fn new(
        issuer: AgentCapabilityIssuer,
        reservation: crate::embedded_server::ManualExecutionHandoffReservation,
    ) -> Self {
        Self {
            issuer,
            reservation,
            state: CloseHandoffState::Pending,
        }
    }

    fn commit_and_hold(&mut self) -> bool {
        // Child exit is the irreversible point. Even if the registry reports
        // that the reservation vanished, Drop must never restore its bearer.
        self.state = CloseHandoffState::Committed;
        self.issuer
            .commit_manual_execution_handoff(&self.reservation)
    }

    fn release_committed(&mut self) -> bool {
        if self.state != CloseHandoffState::Committed {
            return false;
        }
        let released = self
            .issuer
            .release_manual_execution_handoff(&self.reservation);
        if released {
            self.state = CloseHandoffState::Settled;
        }
        released
    }

    fn is_committed(&self) -> bool {
        self.state == CloseHandoffState::Committed
    }

    fn rollback(&mut self) -> bool {
        let rolled_back = self
            .issuer
            .rollback_manual_execution_handoff(&self.reservation);
        if rolled_back {
            self.state = CloseHandoffState::Settled;
        }
        rolled_back
    }
}

impl Drop for CloseHandoffReservation {
    fn drop(&mut self) {
        match self.state {
            CloseHandoffState::Pending => {
                if !self
                    .issuer
                    .rollback_manual_execution_handoff(&self.reservation)
                {
                    tracing::warn!(
                        target: "gwt.pane.teardown",
                        "dropped pane-close handoff reservation could not be rolled back"
                    );
                }
            }
            CloseHandoffState::Committed => {
                if !self
                    .issuer
                    .release_manual_execution_handoff(&self.reservation)
                {
                    tracing::warn!(
                        target: "gwt.pane.teardown",
                        "dropped committed pane-close handoff reservation could not be released"
                    );
                }
            }
            CloseHandoffState::Settled => {}
        }
    }
}

type WindowCloseFinalizerTask = Box<dyn FnOnce() + Send + 'static>;

struct ProcessWindowCloseFinalizer {
    sender: std_mpsc::Sender<WindowCloseFinalizerTask>,
}

static PROCESS_WINDOW_CLOSE_FINALIZER: OnceLock<ProcessWindowCloseFinalizer> = OnceLock::new();

/// Start the process-owned close lane before the GUI accepts any close. A
/// per-close runtime and a newly spawned raw thread can both be unavailable;
/// this already-running worker remains an ownership sink for the accepted
/// finalizer without returning it to Tao.
pub(super) fn initialize_process_window_close_finalizer() -> std::io::Result<()> {
    if PROCESS_WINDOW_CLOSE_FINALIZER.get().is_some() {
        return Ok(());
    }
    let (sender, receiver) = std_mpsc::channel::<WindowCloseFinalizerTask>();
    thread::Builder::new()
        .name("gwt-pane-close-process-worker".to_string())
        .spawn(move || {
            while let Ok(task) = receiver.recv() {
                if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task)) {
                    let detail = panic
                        .downcast_ref::<&str>()
                        .map(|message| (*message).to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".to_string());
                    tracing::error!(
                        target: "gwt.pane.teardown",
                        %detail,
                        "process-owned pane close finalizer panicked"
                    );
                }
            }
        })?;
    if let Err(redundant) =
        PROCESS_WINDOW_CLOSE_FINALIZER.set(ProcessWindowCloseFinalizer { sender })
    {
        // Another initializer won the race. Dropping this sender lets its
        // redundant worker observe channel closure and exit.
        drop(redundant);
    }
    Ok(())
}

fn enqueue_process_window_close_finalizer(task: WindowCloseFinalizerTask) -> Result<(), ()> {
    let Some(dispatcher) = PROCESS_WINDOW_CLOSE_FINALIZER.get() else {
        return Err(());
    };
    dispatcher
        .sender
        .send(with_close_finalizer_test_home(task))
        .map_err(|_| ())
}

#[cfg(test)]
fn with_close_finalizer_test_home(task: WindowCloseFinalizerTask) -> WindowCloseFinalizerTask {
    let gwt_home = gwt_core::test_support::gwt_home_override();
    Box::new(move || {
        let _gwt_home = gwt_home
            .as_ref()
            .map(gwt_core::test_support::ScopedGwtHome::set);
        task();
    })
}

#[cfg(not(test))]
fn with_close_finalizer_test_home(task: WindowCloseFinalizerTask) -> WindowCloseFinalizerTask {
    task
}

#[cfg(test)]
thread_local! {
    static FORCE_CLOSE_FINALIZER_THREAD_SPAWN_FAILURE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static CLOSE_FINALIZER_BEFORE_DURABLE_CLEANUP_TEST_HOOK:
        std::cell::RefCell<Option<WindowCloseFinalizerTask>> = const {
            std::cell::RefCell::new(None)
        };
}

#[cfg(test)]
pub(super) struct CloseFinalizerThreadSpawnFailureGuard {
    previous: bool,
}

#[cfg(test)]
impl Drop for CloseFinalizerThreadSpawnFailureGuard {
    fn drop(&mut self) {
        FORCE_CLOSE_FINALIZER_THREAD_SPAWN_FAILURE.with(|forced| forced.set(self.previous));
    }
}

#[cfg(test)]
pub(super) fn force_close_finalizer_thread_spawn_failure_for_test(
) -> CloseFinalizerThreadSpawnFailureGuard {
    let previous = FORCE_CLOSE_FINALIZER_THREAD_SPAWN_FAILURE.with(|forced| forced.replace(true));
    CloseFinalizerThreadSpawnFailureGuard { previous }
}

#[cfg(test)]
pub(super) fn hold_process_close_finalizer_worker_for_test() -> std_mpsc::Sender<()> {
    initialize_process_window_close_finalizer().expect("start process close finalizer worker");
    let (started_sender, started_receiver) = std_mpsc::channel();
    let (release_sender, release_receiver) = std_mpsc::channel();
    enqueue_process_window_close_finalizer(Box::new(move || {
        let _ = started_sender.send(());
        let _ = release_receiver.recv();
    }))
    .unwrap_or_else(|()| panic!("enqueue process close finalizer barrier"));
    started_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("process close finalizer worker reached barrier");
    release_sender
}

#[cfg(test)]
pub(super) fn set_close_finalizer_before_durable_cleanup_test_hook(
    hook: impl FnOnce() + Send + 'static,
) {
    CLOSE_FINALIZER_BEFORE_DURABLE_CLEANUP_TEST_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(test)]
fn run_close_finalizer_before_durable_cleanup_test_hook() {
    CLOSE_FINALIZER_BEFORE_DURABLE_CLEANUP_TEST_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

fn spawn_window_close_fallback_thread(
    task: WindowCloseFinalizerTask,
) -> std::io::Result<JoinHandle<()>> {
    #[cfg(test)]
    if FORCE_CLOSE_FINALIZER_THREAD_SPAWN_FAILURE.with(std::cell::Cell::get) {
        return Err(std::io::Error::other(
            "injected close finalizer raw thread spawn failure",
        ));
    }
    thread::Builder::new()
        .name("gwt-pane-close-finalizer".to_string())
        .spawn(with_close_finalizer_test_home(task))
}

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

/// Write one pane payload and wait for the physical submit byte to complete.
/// This is only the physical boundary for autonomous-answer delivery; the
/// provider's UserPromptSubmit hook remains the durable acknowledgment. Run it
/// through [`super::BlockingTaskSpawner`] rather than the application loop.
pub(super) fn write_pane_input_and_submit_blocking(
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
    thread::sleep(PANE_SUBMIT_SETTLE);
    reservation
        .write_input(submit.to_string().as_bytes())
        .map_err(|error| error.to_string())
}

/// Issue #3705 AC-3: name the pane whose teardown stalled so a hung
/// `pane.*` channel can be diagnosed from `~/.gwt/logs/` without guessing.
pub(crate) fn pane_teardown_stall_message(window_id: &str, stage: &str, elapsed_ms: u64) -> String {
    format!("PTY teardown stalled: window_id={window_id} stage={stage} elapsed_ms={elapsed_ms}")
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
            Ok(()) => {
                if gwt::window_state::is_approval_resolution_input(text) {
                    self.begin_runtime_approval_resolution(window_id);
                }
                vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::PaneSendResult {
                        ok: true,
                        window_id: Some(window_id.to_string()),
                        error: None,
                    },
                )]
            }
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
            Ok(()) => {
                if gwt::window_state::is_approval_resolution_input(data) {
                    self.begin_runtime_approval_resolution(id);
                }
                Vec::new()
            }
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
                    previous.revoke_input_generation();
                    let window_id = id.to_string();
                    if let Err(_error) = thread::Builder::new()
                        .name("gwt-pty-generation-barrier".to_string())
                        .spawn(move || previous.invalidate_input_generation())
                    {
                        tracing::warn!(
                            target: "gwt_input_trace",
                            window_id = %window_id,
                            stage = "registry_replacement_barrier_spawn_failed",
                            outcome = "revoked_without_background_barrier",
                            "failed to spawn replaced PTY generation barrier"
                        );
                    }
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

    /// Issue #3783: accept a window close by detaching all process-local
    /// ownership first, then run every potentially blocking lifecycle step on
    /// one background finalizer. In particular, the GUI event loop must never
    /// wait for the PTY writer barrier, durable execution leases, child reap,
    /// or the repo-global WorkItems lock before returning `PaneCloseResult`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn queue_window_close_finalizer(
        &mut self,
        window_id: &str,
        close_project_root: Option<std::path::PathBuf>,
        closing_session_id: Option<String>,
        pm_close: bool,
        notify_issue_monitor: bool,
        self_close_ticket: Option<crate::AgentSelfCloseCapabilityTicket>,
        closing_window_generation: Option<u64>,
    ) {
        if let Err(error) = initialize_process_window_close_finalizer() {
            tracing::error!(
                target: "gwt.pane.teardown",
                window_id,
                %error,
                "process-owned close finalizer worker was unavailable"
            );
        }
        enum CloseSessionAuthority {
            Exact {
                identity: Box<gwt_agent::SessionExecutionIdentity>,
                incarnation: Option<u64>,
            },
            Legacy(Box<gwt_agent::Session>),
            Unavailable(String),
        }

        let mut runtime = self.runtimes.remove(window_id);
        let closing_pty = runtime.as_ref().map(|runtime| Arc::clone(&runtime.pty));
        if let Some(runtime) = runtime.as_ref() {
            runtime.pty.revoke_input_generation();
        }
        let session = self.active_agent_sessions.remove(window_id);
        let project_root = close_project_root.or_else(|| {
            session.as_ref().and_then(|session| {
                self.tab(&session.tab_id)
                    .map(|tab| tab.project_root.clone())
            })
        });
        // The close ACK boundary is process-local only. The Launch Wizard
        // cache is the in-memory Session snapshot populated off-thread; all
        // durable Session reads and CAS persistence remain in the finalizer.
        let cached_session = session.as_ref().and_then(|active| {
            self.launch_wizard_cache
                .session_by_id(&active.session_id)
                .cloned()
        });
        let capability_token = self.agent_capability_tokens.remove(window_id);
        let capability_issuer = self.agent_capability_issuer.clone();
        let capability_binding = match (
            capability_issuer.as_ref(),
            self_close_ticket.as_ref(),
            capability_token.as_deref(),
        ) {
            (Some(issuer), Some(ticket), _) => issuer.self_close_active_execution_binding(ticket),
            (Some(issuer), None, Some(token)) => issuer.active_execution_binding_for_token(token),
            _ => None,
        };
        // Natural process exit has no requesting bearer. Its authority is the
        // captured local incarnation plus the exact durable Session/runtime
        // lease below; a present manual/self-close bearer must still match.
        let exact_local_runtime_without_bearer =
            runtime.is_some() && self_close_ticket.is_none() && capability_token.is_none();
        let session_authority = session.as_ref().map(|active| match cached_session.clone() {
            Some(cached) => match gwt_agent::SessionExecutionIdentity::from_session(&cached) {
                Ok(Some(identity))
                    if capability_binding.as_ref() == Some(&identity.execution_binding)
                        || exact_local_runtime_without_bearer =>
                {
                    CloseSessionAuthority::Exact {
                        identity: Box::new(identity),
                        incarnation: runtime.as_ref().map(|runtime| runtime.incarnation),
                    }
                }
                Ok(Some(_)) => CloseSessionAuthority::Unavailable(
                    "in-memory capability binding did not match the cached Session generation"
                        .to_string(),
                ),
                Ok(None) if self_close_ticket.is_none() => {
                    CloseSessionAuthority::Legacy(Box::new(cached))
                }
                Ok(None) => CloseSessionAuthority::Unavailable(
                    "accepted self-close had no exact execution binding".to_string(),
                ),
                Err(error) => CloseSessionAuthority::Unavailable(error),
            },
            None => CloseSessionAuthority::Unavailable(format!(
                "Session {} was unavailable in the in-memory launch cache",
                active.session_id
            )),
        });
        let exact_terminal = session_authority
            .as_ref()
            .and_then(|authority| match authority {
                CloseSessionAuthority::Exact {
                    identity,
                    incarnation,
                } => Some((identity.clone(), *incarnation)),
                CloseSessionAuthority::Legacy(_) | CloseSessionAuthority::Unavailable(_) => None,
            });
        let mut exact_handoff = exact_terminal.as_ref().map(|(identity, _)| {
            match (
                capability_issuer.clone(),
                self_close_ticket.as_ref(),
                capability_token.as_deref(),
            ) {
                (Some(issuer), Some(ticket), _) => issuer
                    .begin_self_close_manual_execution_handoff(ticket, &identity.execution_binding)
                    .map(|reservation| Some(CloseHandoffReservation::new(issuer, reservation))),
                (Some(issuer), None, Some(token)) => issuer
                    .begin_manual_execution_handoff(token, &identity.execution_binding)
                    .map(|reservation| Some(CloseHandoffReservation::new(issuer, reservation))),
                _ => Ok(None),
            }
        });
        if exact_handoff.as_ref().is_some_and(Result::is_err) || exact_terminal.is_none() {
            if let Some(issuer) = capability_issuer.as_ref() {
                match (self_close_ticket.as_ref(), capability_token.as_deref()) {
                    (Some(ticket), _) => {
                        issuer.finish_self_close(ticket);
                    }
                    (None, Some(token)) => {
                        issuer.revoke_token(token);
                    }
                    (None, None) => {}
                }
            }
        }
        if let Some(session) = session.as_ref() {
            self.launch_wizard_cache.mark_stopped(&session.session_id);
            self.mark_cached_active_work_session_stopped(
                &session.tab_id,
                &session.session_id,
                window_id,
            );
        }
        self.remove_window_state_tracking(window_id);
        self.window_details.remove(window_id);

        let close_recorded_at = chrono::Utc::now();
        let pause_task = session.as_ref().and_then(|session| {
            project_root.as_ref().and_then(|project_root| {
                Self::paused_work_record_task(project_root, session, close_recorded_at)
            })
        });
        let pty_writers = Arc::clone(&self.pty_writers);
        let sessions_dir = self.sessions_dir.clone();
        let proxy = self.proxy.clone();
        let window_lifecycle_generations = Arc::clone(&self.window_lifecycle_generations);
        let window_id = window_id.to_string();
        let scheduler_window_id = window_id.clone();
        let task: Box<dyn FnOnce() + Send + 'static> = Box::new(move || {
            let started = Instant::now();
            let mut finalizer_ok = true;
            tracing::info!(
                target: "gwt.pane.teardown",
                window_id = %window_id,
                stage = "close_finalizer",
                outcome = "starting",
                "starting detached pane close finalizer"
            );

            let monitor_close_target = if notify_issue_monitor {
                match project_root.as_deref() {
                    Some(project_root)
                        if window_lifecycle_generation_is_current(
                            &window_lifecycle_generations,
                            &window_id,
                            closing_window_generation,
                        ) => Self::capture_issue_monitor_window_close_target_in_background(
                            project_root,
                            &window_id,
                        ),
                    Some(_) => Ok(None),
                    None => Err(
                        gwt::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
                            "no owning project is available for window close".to_string(),
                        ),
                    ),
                }
            } else {
                Ok(None)
            };

            let writer_to_invalidate = match pty_writers.write() {
                Ok(mut writers) => {
                    let writer = closing_pty.as_ref().and_then(|closing_pty| {
                        writers
                            .get(&window_id)
                            .filter(|current| Arc::ptr_eq(current, closing_pty))
                            .cloned()
                    });
                    if writer.is_some() {
                        writers.remove(&window_id);
                    }
                    drop(writers);
                    closing_pty.clone().or(writer)
                }
                Err(_) => {
                    finalizer_ok = false;
                    tracing::warn!(
                        target: "gwt_input_trace",
                        window_id = %window_id,
                        stage = "close_finalizer_registry_deregister_poisoned",
                        outcome = "registry_lock_failed",
                        "failed to deregister PTY writer in close finalizer"
                    );
                    closing_pty.clone()
                }
            };

            let mut local_process_exited = runtime.is_none();
            let mut terminal_persisted = session.is_none();
            if let Some(pty) = runtime.as_ref().map(|runtime| Arc::clone(&runtime.pty)) {
                let kill_and_reap = || -> std::io::Result<bool> {
                    if pty
                        .try_wait()
                        .map_err(|error| std::io::Error::other(error.to_string()))?
                        .is_some()
                    {
                        return Ok(true);
                    }
                    tracing::info!(
                        target: "gwt.pane.teardown",
                        window_id = %window_id,
                        stage = "pty_kill",
                        outcome = "starting",
                        "starting detached PTY teardown stage"
                    );
                    let kill_started = Instant::now();
                    let kill_result = pty.kill();
                    let kill_elapsed_ms =
                        u64::try_from(kill_started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    tracing::info!(
                        target: "gwt.pane.teardown",
                        window_id = %window_id,
                        stage = "pty_kill",
                        elapsed_ms = kill_elapsed_ms,
                        ok = kill_result.is_ok(),
                        outcome = if kill_result.is_ok() { "completed" } else { "failed" },
                        "detached PTY teardown stage completed"
                    );
                    kill_result.map_err(|error| std::io::Error::other(error.to_string()))?;
                    let deadline = Instant::now() + Duration::from_secs(2);
                    loop {
                        if pty
                            .try_wait()
                            .map_err(|error| std::io::Error::other(error.to_string()))?
                            .is_some()
                        {
                            return Ok(true);
                        }
                        if Instant::now() >= deadline {
                            return Ok(false);
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                };

                match (exact_terminal.as_ref(), exact_handoff.as_mut()) {
                    (Some((identity, Some(incarnation))), Some(Ok(_handoff))) => {
                        let proof = gwt_agent::ManualLaunchRuntimeProof {
                            host_pid: std::process::id(),
                            runtime_incarnation: *incarnation,
                        };
                        let stopped =
                            gwt::cli::execution_state::with_exact_active_manual_runtime_lease(
                                &sessions_dir,
                                identity,
                                proof,
                                false,
                                || {
                                    if !kill_and_reap()? {
                                        return Err(std::io::Error::other(
                                        "detached PTY did not exit before the finalizer deadline",
                                    ));
                                    }
                                    if !gwt_agent::persist_session_terminal_status_if_execution_identity_matches_under_lease(
                                    &sessions_dir,
                                    identity,
                                    *incarnation,
                                    gwt_agent::AgentStatus::Stopped,
                                )? {
                                    return Err(std::io::Error::other(
                                        "holder Session changed before terminal proof persistence",
                                    ));
                                }
                                    if !gwt_agent::persist_session_restore_window_on_startup_if_execution_identity_matches_under_lease(
                                        &sessions_dir,
                                        identity,
                                        false,
                                    )? {
                                        return Err(std::io::Error::other(
                                            "holder Session changed before startup-restore persistence",
                                        ));
                                    }
                                    Ok(())
                                },
                            );
                        terminal_persisted = matches!(&stopped, Ok(Some(())));
                        local_process_exited = if terminal_persisted {
                            true
                        } else {
                            match pty.try_wait() {
                                Ok(Some(_)) => true,
                                Ok(None) => kill_and_reap().unwrap_or(false),
                                Err(_) => false,
                            }
                        };
                        finalizer_ok &= terminal_persisted;
                        if let Err(error) = stopped {
                            tracing::warn!(
                                target: "gwt.pane.teardown",
                                window_id = %window_id,
                                %error,
                                "detached exact-holder finalizer failed"
                            );
                        }
                    }
                    (Some(_), Some(Err(error))) => {
                        finalizer_ok = false;
                        tracing::warn!(
                            target: "gwt.pane.teardown",
                            window_id = %window_id,
                            %error,
                            "exact-holder capability fence failed; stopping only the local PTY"
                        );
                        local_process_exited = kill_and_reap().unwrap_or(false);
                    }
                    _ => match kill_and_reap() {
                        Ok(exited) => {
                            local_process_exited = exited;
                            finalizer_ok &= exited;
                        }
                        Err(error) => {
                            finalizer_ok = false;
                            tracing::warn!(
                                target: "gwt.pane.teardown",
                                window_id = %window_id,
                                %error,
                                "detached PTY kill/reap failed"
                            );
                        }
                    },
                }
            }

            if runtime.is_none() {
                if let Some((identity, None)) = exact_terminal.as_ref() {
                    if let Some(Ok(_handoff)) = exact_handoff.as_mut() {
                        terminal_persisted =
                            gwt_agent::persist_session_restore_window_on_startup_if_execution_identity_matches(
                            &sessions_dir,
                            identity,
                            false,
                        )
                        .unwrap_or(false);
                    } else {
                        terminal_persisted = false;
                    }
                    finalizer_ok &= terminal_persisted;
                }
            }

            // Once the child is gone, invalidate the predecessor capability
            // irreversibly but retain the committed reservation as a fence.
            // Do this before any later barrier or durable cleanup can unwind;
            // a non-exited child is the only case that restores a normal
            // handoff bearer.
            if let Some(Ok(Some(handoff))) = exact_handoff.as_mut() {
                if local_process_exited {
                    if !handoff.commit_and_hold() {
                        finalizer_ok = false;
                        tracing::warn!(
                            target: "gwt.pane.teardown",
                            window_id = %window_id,
                            "detached exact-holder capability fence could not be committed"
                        );
                    }
                } else if !handoff.rollback() {
                    finalizer_ok = false;
                    tracing::warn!(
                        target: "gwt.pane.teardown",
                        window_id = %window_id,
                        "detached exact-holder capability rollback failed"
                    );
                }
            }

            // Logical revocation happened synchronously at close acceptance.
            // Wait for the physical writer barrier only after kill/reap, so a
            // write blocked on the child cannot prevent the child from being
            // terminated and stall every later cleanup stage.
            if let Some(writer) = writer_to_invalidate.as_ref() {
                writer.invalidate_input_generation();
            }

            if local_process_exited && !terminal_persisted {
                match session_authority.as_ref() {
                    Some(CloseSessionAuthority::Legacy(expected)) => {
                        let mut stopped = expected.clone();
                        stopped.update_status(gwt_agent::AgentStatus::Stopped);
                        stopped.restore_window_on_startup = false;
                        terminal_persisted = stopped
                            .save_if_unchanged(&sessions_dir, expected)
                            .unwrap_or(false);
                        finalizer_ok &= terminal_persisted;
                    }
                    Some(CloseSessionAuthority::Unavailable(error)) => {
                        finalizer_ok = false;
                        tracing::warn!(
                            target: "gwt.pane.teardown",
                            window_id = %window_id,
                            %error,
                            "predecessor Session authority was unavailable; skipped durable close projection"
                        );
                    }
                    Some(CloseSessionAuthority::Exact { .. }) | None => {}
                }
            }

            #[cfg(test)]
            run_close_finalizer_before_durable_cleanup_test_hook();
            let cleanup_generation_is_current = window_lifecycle_generation_is_current(
                &window_lifecycle_generations,
                &window_id,
                closing_window_generation,
            );
            if let Some(session) = session
                .as_ref()
                .filter(|_| terminal_persisted && cleanup_generation_is_current)
            {
                let ephemeral = Self::session_uses_ephemeral_worktree_for_project(
                    project_root.as_deref(),
                    session,
                );
                if ephemeral {
                    Self::finalize_ephemeral_worktree_for_project(project_root.as_deref(), session);
                } else {
                    if let Some(task) = pause_task {
                        task();
                    }
                    if let Some(project_root) = project_root.as_ref() {
                        if let Err(error) =
                            gwt_core::workspace_projection::mark_workspace_agent_stopped(
                                project_root,
                                &session.session_id,
                                Some(&session.window_id),
                            )
                        {
                            finalizer_ok = false;
                            tracing::warn!(
                                error = %error,
                                project_root = %project_root.display(),
                                session_id = %session.session_id,
                                window_id = %session.window_id,
                                "failed to clean stopped Agent from Workspace projection"
                            );
                        }
                    }
                }
            }

            if let Some(Ok(Some(handoff))) = exact_handoff.as_mut() {
                if handoff.is_committed() && !handoff.release_committed() {
                    finalizer_ok = false;
                    tracing::warn!(
                        target: "gwt.pane.teardown",
                        window_id = %window_id,
                        "detached exact-holder capability fence could not be released"
                    );
                }
            }

            if let Some(runtime) = runtime.as_mut() {
                if let Some(handle) = runtime.output_thread.take() {
                    finalizer_ok &= handle.join().is_ok();
                }
                if let Some(handle) = runtime.status_thread.take() {
                    finalizer_ok &= handle.join().is_ok();
                }
            }

            let (pm_deregistered, pm_status) =
                match (project_root.as_deref(), closing_session_id.as_deref()) {
                    (Some(project_root), Some(session_id)) => {
                        Self::deregister_pm_for_closed_window_in_background(
                            project_root,
                            session_id,
                        )
                    }
                    _ => (false, None),
                };
            let monitor_result = if notify_issue_monitor {
                match (project_root.as_deref(), monitor_close_target) {
                    (Some(project_root), Ok(Some(target)))
                        if window_lifecycle_generation_is_current(
                            &window_lifecycle_generations,
                            &window_id,
                            closing_window_generation,
                        ) =>
                    {
                        Self::finalize_issue_monitor_window_close_in_background(
                            project_root,
                            &target,
                        )
                    }
                    (_, Ok(_)) => WindowCloseMonitorResult::Noop,
                    (_, Err(error)) => WindowCloseMonitorResult::Failed(error),
                }
            } else {
                WindowCloseMonitorResult::Noop
            };
            settle_window_lifecycle_generation(
                &window_lifecycle_generations,
                &window_id,
                closing_window_generation,
            );
            proxy.send(UserEvent::WindowCloseFinalized {
                window_id: window_id.clone(),
                project_root: project_root.clone(),
                closing_session_id: closing_session_id.clone(),
                pm_close,
                pm_deregistered,
                pm_status,
                monitor_result,
            });
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            tracing::info!(
                target: "gwt.pane.teardown",
                window_id = %window_id,
                stage = "close_finalizer",
                elapsed_ms,
                outcome = if finalizer_ok { "completed" } else { "failed" },
                "detached pane close finalizer completed"
            );
        });

        // Keep a recoverable copy of the task so a saturated or unavailable
        // runtime cannot turn an accepted close into leaked process ownership.
        let pending = Arc::new(Mutex::new(Some(task)));
        let scheduled = Arc::clone(&pending);
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            if let Some(task) = scheduled
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                task();
            }
        }) {
            tracing::warn!(
                target: "gwt.pane.teardown",
                window_id = %scheduler_window_id,
                %error,
                "close finalizer scheduler failed; using a detached thread"
            );
            let fallback = Arc::clone(&pending);
            if let Err(fallback_error) = spawn_window_close_fallback_thread(Box::new(move || {
                if let Some(task) = fallback
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    task();
                }
            })) {
                tracing::error!(
                    target: "gwt.pane.teardown",
                    window_id = %scheduler_window_id,
                    %fallback_error,
                    "close finalizer fallback thread could not be started; using process-owned worker"
                );
                let task = pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
                if let Some(task) = task {
                    if enqueue_process_window_close_finalizer(task).is_err() {
                        tracing::error!(
                            target: "gwt.pane.teardown",
                            window_id = %scheduler_window_id,
                            "process-owned close finalizer queue disconnected after close acceptance"
                        );
                    }
                }
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
            Self::detach_runtime_stop_threads(window_id, threads);
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
        let pty = runtime.pty.clone();
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
                    tracing::info!(
                        target: "gwt.pane.teardown",
                        window_id,
                        stage = "exact_pty_kill",
                        outcome = "starting",
                        "starting exact-holder PTY teardown stage"
                    );
                    let kill_started = Instant::now();
                    let kill_result = pty.kill();
                    let kill_elapsed_ms =
                        u64::try_from(kill_started.elapsed().as_millis()).unwrap_or(u64::MAX);
                    tracing::info!(
                        target: "gwt.pane.teardown",
                        window_id,
                        stage = "exact_pty_kill",
                        elapsed_ms = kill_elapsed_ms,
                        ok = kill_result.is_ok(),
                        outcome = if kill_result.is_ok() { "completed" } else { "failed" },
                        "exact-holder PTY teardown stage completed"
                    );
                    kill_result.map_err(|error| std::io::Error::other(error.to_string()))?;
                    // Issue #3705: never sleep on the GUI event loop waiting
                    // for reap. Persist under lease only when the child is
                    // already gone; otherwise `start_window_runtime_stop`
                    // finishes the proof on a background thread.
                    let exited = pty
                        .try_wait()
                        .map_err(|error| std::io::Error::other(error.to_string()))?;
                    if exited.is_some()
                        && !gwt_agent::persist_session_terminal_status_if_execution_identity_matches_under_lease(
                            &sessions_dir,
                            expected_session,
                            expected_incarnation,
                            gwt_agent::AgentStatus::Stopped,
                        )?
                    {
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
        Self::detach_runtime_stop_threads(window_id, threads);
        self.mark_agent_session_stopped_with_persistence(window_id, false);
        Ok(())
    }

    fn stop_window_runtime_inner(&mut self, window_id: &str, mark_session_stopped: bool) {
        let threads = self.start_window_runtime_stop(window_id, mark_session_stopped);
        Self::detach_runtime_stop_threads(window_id, threads);
    }

    fn start_window_runtime_stop(
        &mut self,
        window_id: &str,
        mark_session_stopped: bool,
    ) -> RuntimeStopThreads {
        tracing::info!(
            target: "gwt.pane.teardown",
            window_id,
            "starting PTY teardown"
        );
        let exact_terminal = self
            .active_agent_sessions
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
            });
        self.remove_window_state_tracking(window_id);
        self.deregister_pty_writer(window_id);
        let mut threads = RuntimeStopThreads {
            output_thread: None,
            status_thread: None,
        };
        let mut exact_terminal_persisted = false;
        if let Some(mut runtime) = self.runtimes.remove(window_id) {
            let pty = runtime.pty.clone();
            tracing::info!(
                target: "gwt.pane.teardown",
                window_id,
                stage = "pty_kill",
                outcome = "starting",
                "starting PTY teardown stage"
            );
            let kill_started = Instant::now();
            let kill_result = pty.kill();
            let kill_elapsed_ms =
                u64::try_from(kill_started.elapsed().as_millis()).unwrap_or(u64::MAX);
            tracing::info!(
                target: "gwt.pane.teardown",
                window_id,
                stage = "pty_kill",
                elapsed_ms = kill_elapsed_ms,
                ok = kill_result.is_ok(),
                outcome = if kill_result.is_ok() { "completed" } else { "failed" },
                "PTY teardown stage completed"
            );
            if let Err(error) = &kill_result {
                tracing::warn!(
                    target: "gwt.pane.teardown",
                    window_id,
                    stage = "pty_kill",
                    elapsed_ms = kill_elapsed_ms,
                    outcome = "failed",
                    %error,
                    "PTY teardown stage failed"
                );
            }
            if kill_elapsed_ms >= 500 {
                tracing::warn!(
                    target: "gwt.pane.teardown",
                    window_id,
                    elapsed_ms = kill_elapsed_ms,
                    "{}",
                    pane_teardown_stall_message(window_id, "pty_kill", kill_elapsed_ms)
                );
            }
            if let Some((identity, incarnation)) = exact_terminal.clone() {
                let exited = pty.try_wait().ok().flatten().is_some();
                if exited {
                    exact_terminal_persisted =
                        gwt_agent::persist_session_terminal_status_if_execution_identity_matches(
                            &self.sessions_dir,
                            &identity,
                            incarnation,
                            gwt_agent::AgentStatus::Stopped,
                        )
                        .unwrap_or(false);
                } else {
                    let sessions_dir = self.sessions_dir.clone();
                    let window_id = window_id.to_string();
                    thread::spawn(move || {
                        let started = Instant::now();
                        let deadline = Instant::now() + Duration::from_secs(2);
                        let mut exited = false;
                        while Instant::now() < deadline {
                            if pty.try_wait().ok().flatten().is_some() {
                                exited = true;
                                break;
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                        let elapsed_ms =
                            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                        if exited {
                            let _ = gwt_agent::persist_session_terminal_status_if_execution_identity_matches(
                                &sessions_dir,
                                &identity,
                                incarnation,
                                gwt_agent::AgentStatus::Stopped,
                            );
                        } else {
                            tracing::warn!(
                                target: "gwt.pane.teardown",
                                window_id = %window_id,
                                elapsed_ms,
                                "{}",
                                pane_teardown_stall_message(&window_id, "process_exit", elapsed_ms)
                            );
                        }
                    });
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

    fn detach_runtime_stop_threads(window_id: &str, mut threads: RuntimeStopThreads) {
        if threads.output_thread.is_none() && threads.status_thread.is_none() {
            return;
        }
        let window_id = window_id.to_string();
        thread::spawn(move || {
            let started = Instant::now();
            if let Some(handle) = threads.output_thread.take() {
                let _ = handle.join();
            }
            if let Some(handle) = threads.status_thread.take() {
                let _ = handle.join();
            }
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            if elapsed_ms >= 500 {
                tracing::warn!(
                    target: "gwt.pane.teardown",
                    window_id = %window_id,
                    elapsed_ms,
                    "{}",
                    pane_teardown_stall_message(&window_id, "join", elapsed_ms)
                );
            }
        });
    }

    fn join_runtime_stop_threads_blocking(mut threads: RuntimeStopThreads) {
        if let Some(handle) = threads.output_thread.take() {
            // Shutdown is allowed to wait briefly so reader threads release
            // their Pane Arcs before process exit. Cap the wait so a stuck
            // `read` cannot hang quit. Issue #3705: pane.close must not use
            // this path — it detaches instead.
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
            Self::join_runtime_stop_threads_blocking,
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

    /// Issue #3705 AC-3: a stalled teardown must name the pane, not just say
    /// that "something" blocked, so logs can pinpoint which close froze pane.*.
    #[test]
    fn pane_teardown_stall_message_names_the_window() {
        let message = super::pane_teardown_stall_message("tab-1::agent-4", "join", 1500);
        assert!(
            message.contains("tab-1::agent-4"),
            "stall log must name the pane, got: {message}"
        );
        assert!(message.contains("join"), "stall log must name the stage");
        assert!(
            message.contains("1500"),
            "stall log must include elapsed_ms"
        );
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
