//! Rust GUI adapters for the authenticated project owner. Terminal state is a read-only cache.
use std::sync::{mpsc as std_mpsc, Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::window::PeerPaneCloseReply;
use base64::Engine as _;

use super::{
    combined_window_id, AppRuntime, BackendEvent, ClientId, OutboundEvent, UserEvent,
    WindowCloseMonitorResult, WindowProcessStatus,
};

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
pub(super) use gwt::project_runtime::agent_dispatch::{
    drive_verified_pane_submit, pane_input_body_bytes, split_pane_submit, VerifiedPaneSubmitOutcome,
};

pub(crate) use gwt::session_finalizer::pane_teardown_stall_message;

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
    fn remote_command_refusal(
        &self,
        id: &str,
        kind: &super::RemoteRuntimeCommandKind,
        error: String,
    ) -> Vec<OutboundEvent> {
        if let super::RemoteRuntimeCommandKind::Input {
            reply_client: Some(client_id),
        } = kind
        {
            return vec![OutboundEvent::reply(
                client_id.clone(),
                BackendEvent::PaneSendResult {
                    ok: false,
                    window_id: Some(id.into()),
                    error: Some(error),
                },
            )];
        }
        self.status_events(id.to_string(), WindowProcessStatus::Error, Some(error))
    }

    pub(super) fn queue_remote_runtime_command(
        &mut self,
        id: &str,
        command: gwt::project_runtime::events::OwnerCommand,
        kind: super::RemoteRuntimeCommandKind,
    ) -> Vec<OutboundEvent> {
        let Some(runtime) = self.runtimes.get(id) else {
            return self.remote_command_refusal(
                id,
                &kind,
                "The project execution is unavailable.".into(),
            );
        };
        let Some(context) = self
            .window_lookup
            .get(id)
            .and_then(|address| self.project_context(&address.tab_id))
        else {
            return Vec::new();
        };
        let Some(state) = self.project_state(&context) else {
            return Vec::new();
        };
        if state.project_owner_identity.as_ref() != Some(&runtime.owner_identity)
            || runtime.owner_pane_id.is_empty()
        {
            return self.remote_command_refusal(
                id,
                &kind,
                "Refresh the project connection and try again.".into(),
            );
        }
        let approval_resolution_started = match &command {
            gwt::project_runtime::events::OwnerCommand::Input { data, .. } => {
                gwt::window_state::is_approval_resolution_input(&String::from_utf8_lossy(data))
            }
            gwt::project_runtime::events::OwnerCommand::SendInput { text, .. } => {
                gwt::window_state::is_approval_resolution_input(text)
            }
            _ => false,
        };
        let connection_id = state.project_owner_connection_id;
        let identity = runtime.owner_identity.clone();
        let incarnation = runtime.incarnation;
        let client = runtime.client.clone();
        let id = id.to_string();
        let failure_id = id.clone();
        let failure_kind = kind.clone();
        let proxy = self.proxy.clone();
        if approval_resolution_started {
            self.begin_runtime_approval_resolution(&id);
        }
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|runtime| {
                    runtime.block_on(async {
                        tokio::time::timeout(Duration::from_secs(120), client.command(command))
                            .await
                            .map_err(|_| "project command timed out".to_string())?
                            .map(|value| {
                                value
                                    .get("has_unsent_user_input")
                                    .and_then(serde_json::Value::as_bool)
                            })
                    })
                });
            proxy.send(UserEvent::RemoteRuntimeCommandComplete(Box::new(
                super::RemoteRuntimeCommandCompletion {
                    context,
                    connection_id,
                    identity,
                    id,
                    incarnation,
                    kind,
                    approval_resolution_started,
                    result,
                },
            )));
        }) {
            if approval_resolution_started {
                self.cancel_runtime_approval_resolution(&failure_id);
            }
            return self.remote_command_refusal(&failure_id, &failure_kind, error.to_string());
        }
        Vec::new()
    }

    pub(crate) fn handle_remote_runtime_command_complete(
        &mut self,
        completion: super::RemoteRuntimeCommandCompletion,
    ) -> Vec<OutboundEvent> {
        let super::RemoteRuntimeCommandCompletion {
            context,
            connection_id,
            identity,
            id,
            incarnation,
            kind,
            approval_resolution_started,
            result,
        } = completion;
        if !self.project_owner_event_is_current(&context, connection_id, &identity)
            || !self.remote_runtime_is_current(&context, &identity, &id, incarnation)
        {
            return Vec::new();
        }
        let unsent = match result {
            Err(error) => {
                if approval_resolution_started {
                    self.cancel_runtime_approval_resolution(&id);
                }
                return self.remote_command_refusal(&id, &kind, error);
            }
            Ok(unsent) => unsent,
        };
        if let Some(unsent) = unsent {
            self.runtimes
                .get(&id)
                .expect("fenced remote input")
                .has_unsent_user_input
                .store(unsent, std::sync::atomic::Ordering::Release);
            if !unsent {
                self.flush_pending_pm_wake(&id);
            }
        }
        match kind {
            super::RemoteRuntimeCommandKind::Input {
                reply_client: Some(client_id),
            } => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PaneSendResult {
                    ok: true,
                    window_id: Some(id),
                    error: None,
                },
            )],
            super::RemoteRuntimeCommandKind::Stop => {
                if let Some(runtime) = self.runtimes.get_mut(&id) {
                    runtime.exit_confirmed = true;
                    runtime.status = WindowProcessStatus::Stopped;
                    runtime.status_revision = runtime.status_revision.saturating_add(1);
                }
                self.apply_owner_status_projection(&id, WindowProcessStatus::Stopped, None)
            }
            _ => Vec::new(),
        }
    }

    /// Immediate repair comes from the immutable owner cache; a fresh private snapshot follows.
    pub(crate) fn client_pane_snapshot_repair_events(
        &self,
        client_id: &str,
        pane_ids: &[String],
    ) -> Vec<OutboundEvent> {
        let mut events = Vec::new();
        for id in pane_ids {
            if let Some(runtime) = self.runtimes.get(id) {
                if !runtime.screen_ansi.is_empty() {
                    events.push(
                        OutboundEvent::reply(
                            client_id,
                            BackendEvent::TerminalSnapshot {
                                id: id.clone(),
                                data_base64: base64::engine::general_purpose::STANDARD
                                    .encode(&runtime.screen_ansi),
                            },
                        )
                        .with_terminal_stream_seq(Some(runtime.screen_seq)),
                    );
                }
                events.extend(self.request_owner_snapshot(id, Some(client_id.to_string())));
            }
        }
        events
    }

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

    pub(crate) fn pane_send_input_for_project_events(
        &mut self,
        client_id: ClientId,
        project_key: &gwt_core::repo_hash::ProjectKey,
        session_id: &str,
        text: &str,
    ) -> Vec<OutboundEvent> {
        let target = self
            .tabs
            .iter()
            .filter(|tab| self.project_key_for_tab(&tab.id) == Some(project_key))
            .find_map(|tab| {
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
                    error: Some("no pane bound to this project session".into()),
                },
            )];
        };
        self.pane_send_input_to_window_events(client_id, &window_id, text)
    }

    pub(crate) fn pane_send_input_to_window_events(
        &mut self,
        client_id: ClientId,
        window_id: &str,
        text: &str,
    ) -> Vec<OutboundEvent> {
        let kind = super::RemoteRuntimeCommandKind::Input {
            reply_client: Some(client_id),
        };
        let Some(runtime) = self.runtimes.get(window_id) else {
            return self.remote_command_refusal(
                window_id,
                &kind,
                "no remote pane is installed".into(),
            );
        };
        self.queue_remote_runtime_command(
            window_id,
            gwt::project_runtime::events::OwnerCommand::SendInput {
                id: runtime.owner_pane_id.clone(),
                incarnation: runtime.incarnation,
                text: text.to_string(),
            },
            kind,
        )
    }

    pub(crate) fn terminal_input_events(&mut self, id: &str, data: &str) -> Vec<OutboundEvent> {
        let kind = super::RemoteRuntimeCommandKind::Input { reply_client: None };
        let Some(runtime) = self.runtimes.get_mut(id) else {
            return self.remote_command_refusal(id, &kind, "no remote pane is installed".into());
        };
        // A scheduling hint only. The project owner rechecks its physical
        // composer before every wake and repairs this cache after input ACK.
        for byte in data.bytes() {
            match byte {
                b'\r' | b'\n' | 3 | 4 | 21 => runtime
                    .has_unsent_user_input
                    .store(false, std::sync::atomic::Ordering::Release),
                32..=255 => runtime
                    .has_unsent_user_input
                    .store(true, std::sync::atomic::Ordering::Release),
                _ => {}
            }
        }
        let command = gwt::project_runtime::events::OwnerCommand::Input {
            id: runtime.owner_pane_id.clone(),
            incarnation: runtime.incarnation,
            data: data.as_bytes().to_vec(),
        };
        self.queue_remote_runtime_command(id, command, kind)
    }

    pub(crate) fn queue_window_close_finalizer(
        &mut self,
        window_id: &str,
        close_project_root: Option<std::path::PathBuf>,
        closing_session_id: Option<String>,
        pm_close: bool,
        _notify_issue_monitor: bool,
        _self_close_ticket: Option<crate::AgentSelfCloseCapabilityTicket>,
        closing_window_generation: Option<u64>,
        peer_reply: Option<PeerPaneCloseReply>,
    ) {
        let _ = initialize_process_window_close_finalizer();
        let runtime = self.runtimes.remove(window_id);
        let pending = self.pending_launch_completions.remove(window_id);
        if let Some(pending) = &pending {
            pending
                .current
                .store(false, std::sync::atomic::Ordering::Release);
        }
        self.active_agent_sessions.remove(window_id);
        let window_id = window_id.to_string();
        let scheduler_window_id = window_id.clone();
        let proxy = self.proxy.clone();
        let generations = Arc::clone(&self.window_lifecycle_generations);
        let task: WindowCloseFinalizerTask = Box::new(move || {
            let mut pm_outcome: Option<gwt::project_runtime::pm_delivery::PmLifecycleOutcome> =
                None;
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|executor| {
                    executor.block_on(async {
                        if let Some(pending) = &pending {
                            super::cancel_remote_owner_launch(
                                &pending.owner_pane_id,
                                &pending.operation_id,
                                &pending.client_cell,
                                &pending.cancel_ack,
                            )
                            .await?;
                        }
                        if let Some(runtime) = runtime.filter(|runtime| {
                            !pending.as_ref().is_some_and(|pending| {
                                pending.owner_pane_id == runtime.owner_pane_id
                            })
                        }) {
                            let receipt = tokio::time::timeout(
                                Duration::from_secs(120),
                                runtime.client.command(
                                    gwt::project_runtime::events::OwnerCommand::Close {
                                        id: runtime.owner_pane_id.clone(),
                                        incarnation: runtime.incarnation,
                                    },
                                ),
                            )
                            .await
                            .map_err(|_| "project close timed out".to_string())??;
                            if receipt["id"].as_str() != Some(runtime.owner_pane_id.as_str())
                                || receipt["incarnation"].as_u64() != Some(runtime.incarnation)
                            {
                                return Err("project close receipt identity changed".into());
                            }
                            pm_outcome = receipt
                                .get("pm_lifecycle")
                                .filter(|value| !value.is_null())
                                .map(|value| serde_json::from_value(value.clone()))
                                .transpose()
                                .map_err(|error| error.to_string())?;
                        } else if pending.is_none() {
                            return Err(
                                "exact remote runtime is unavailable; close was not confirmed"
                                    .to_string(),
                            );
                        }
                        Ok(())
                    })
                });
            if let Some(reply) = peer_reply {
                proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                    reply.client_id,
                    BackendEvent::PaneCloseResult {
                        ok: result.is_ok(),
                        window_id: window_id.clone(),
                        reason: result.as_ref().err().cloned(),
                    },
                )]));
            }
            settle_window_lifecycle_generation(&generations, &window_id, closing_window_generation);
            let pm_status = pm_outcome.as_ref().map(|outcome| {
                gwt::project_runtime::pm_delivery::pm_status_from_launch_prefs(
                    &gwt_core::paths::gwt_sessions_dir(),
                    &outcome.prefs,
                    None,
                )
            });
            let pm_deregistered = pm_outcome
                .as_ref()
                .is_some_and(|outcome| outcome.deregistered);
            proxy.send(UserEvent::WindowCloseFinalized {
                window_id,
                project_root: close_project_root,
                closing_session_id,
                pm_close,
                pm_deregistered,
                pm_status,
                monitor_result: match result {
                    Ok(()) => WindowCloseMonitorResult::Noop,
                    Err(error) => WindowCloseMonitorResult::Failed(
                        gwt::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(
                            error,
                        ),
                    ),
                },
            });
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
        if let Some(runtime) = self.runtimes.get(window_id) {
            let command = gwt::project_runtime::events::OwnerCommand::Close {
                id: runtime.owner_pane_id.clone(),
                incarnation: runtime.incarnation,
            };
            let events = self.queue_remote_runtime_command(
                window_id,
                command,
                super::RemoteRuntimeCommandKind::Stop,
            );
            if !events.is_empty() {
                self.proxy.send(UserEvent::Dispatch(events));
            }
        }
    }

    /// GUI shutdown detaches its caches; owner residency performs the exact lifetime drain.
    pub(crate) fn stop_all_runtimes(&mut self) {
        let ids = self.runtimes.keys().cloned().collect();
        self.stop_runtimes_in_shutdown_order(ids);
    }
    pub(super) fn stop_runtimes_in_shutdown_order(&mut self, ids: Vec<String>) {
        stop_all_before_joining(ids, |id| self.runtimes.remove(&id), drop);
    }
}

#[cfg(test)]
mod submit_split_tests {
    use super::{
        drive_verified_pane_submit, pane_input_body_bytes, split_pane_submit,
        VerifiedPaneSubmitOutcome,
    };

    /// Issue #4909 AC-2 / AC-3: the exact bytes the injection layer writes for
    /// a prompt that carries an `@` mention, a leading `/`, and two lines.
    /// With bracketed paste on, the body is one paste and the submit byte is
    /// still a separate write; with it off, the raw path is byte-identical to
    /// before.
    #[test]
    fn injected_body_is_wrapped_as_a_paste_only_when_the_composer_accepts_pastes() {
        let prompt = "/mnt/work/notes.md\n@agent-1 please review\r";
        let body = "/mnt/work/notes.md\n@agent-1 please review";
        assert_eq!(
            pane_input_body_bytes(body, true).expect("safe pasted body"),
            [b"\x1b[200~".as_slice(), body.as_bytes(), b"\x1b[201~"].concat()
        );
        assert_eq!(
            pane_input_body_bytes(body, false).expect("safe raw body"),
            body.as_bytes()
        );

        for bracketed_paste in [true, false] {
            let writes = std::cell::RefCell::new(Vec::<Vec<u8>>::new());
            let outcome = drive_verified_pane_submit(
                prompt,
                bracketed_paste,
                1,
                |bytes| {
                    writes.borrow_mut().push(bytes.to_vec());
                    Ok(())
                },
                |_| {},
                || Ok(true),
            )
            .expect("delivered");
            assert_eq!(
                outcome,
                VerifiedPaneSubmitOutcome::Verified { submit_attempts: 1 }
            );
            let expected_body = if bracketed_paste {
                format!("\x1b[200~{body}\x1b[201~").into_bytes()
            } else {
                body.as_bytes().to_vec()
            };
            assert_eq!(
                *writes.borrow(),
                vec![expected_body, b"\r".to_vec()],
                "bracketed_paste={bracketed_paste}: body in one write, submit byte on its own"
            );
        }
    }

    /// Issue #5839: Windows Codex interprets these as keys even inside a
    /// bracketed paste. Refuse the entire body before staging any bytes.
    #[test]
    fn notification_control_keys_are_refused_before_any_write() {
        for body in ["notice \x14suffix", "notice \x1bORsuffix"] {
            for bracketed_paste in [false, true] {
                let mut writes = Vec::new();
                let result = drive_verified_pane_submit(
                    &format!("{body}\r"),
                    bracketed_paste,
                    1,
                    |bytes| {
                        writes.push(bytes.to_vec());
                        Ok(())
                    },
                    |_| {},
                    || Ok(true),
                );
                assert!(result.is_err(), "control keys must not reach the TUI");
                assert!(writes.is_empty(), "refuse before staging the body");
            }
        }
    }

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
    fn same_operation_recovery_writes_only_submit_and_honors_a_late_ack() {
        let writes = std::cell::RefCell::new(Vec::new());
        let first = drive_verified_pane_submit(
            "protected body\r",
            false,
            2,
            |bytes| {
                writes.borrow_mut().push(bytes.to_vec());
                Ok(())
            },
            |_| {},
            || Ok(false),
        )
        .unwrap();
        assert!(matches!(
            first,
            VerifiedPaneSubmitOutcome::Unverified { .. }
        ));
        let recovered = drive_verified_pane_submit(
            "\r",
            false,
            2,
            |bytes| {
                writes.borrow_mut().push(bytes.to_vec());
                Ok(())
            },
            |_| {},
            || Ok(writes.borrow().len() == 4),
        )
        .unwrap();
        assert_eq!(
            recovered,
            VerifiedPaneSubmitOutcome::Verified { submit_attempts: 1 }
        );
        assert_eq!(
            *writes.borrow(),
            [
                b"protected body".to_vec(),
                b"\r".to_vec(),
                b"\r".to_vec(),
                b"\r".to_vec()
            ]
        );
        drive_verified_pane_submit(
            "\r",
            false,
            2,
            |_| panic!("late acknowledged replay must not write"),
            |_| {},
            || Ok(true),
        )
        .unwrap();
    }

    #[test]
    fn unverified_submit_retries_only_the_carriage_return() {
        let writes = std::cell::RefCell::new(Vec::new());

        let outcome = drive_verified_pane_submit(
            "protected body\r",
            false,
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
            false,
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
            false,
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
