//! Runtime event handlers and daemon publish queue for SPEC-2077 Phase J1.
//!
//! This module owns the already daemon-aware runtime output/status/hook
//! publish path. The extraction is behavior-preserving: it keeps best-effort
//! daemon publish, same-process echo suppression through the existing payload
//! layer, and the local GUI state update path unchanged.

use base64::Engine as _;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::sync::mpsc as std_mpsc;
#[cfg(unix)]
use std::sync::Mutex;

use super::{
    close_window_from_workspace, should_auto_close_agent_window, AppRuntime, BackendEvent,
    OutboundEvent, WindowPreset, WindowProcessStatus,
};

impl AppRuntime {
    pub(crate) fn handle_runtime_output(
        &mut self,
        id: String,
        data: Vec<u8>,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_output_inner(id, data, true)
    }

    pub(crate) fn handle_daemon_runtime_output(
        &mut self,
        id: String,
        data: Vec<u8>,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_output_inner(id, data, false)
    }

    fn handle_runtime_output_inner(
        &mut self,
        id: String,
        data: Vec<u8>,
        publish_to_daemon: bool,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(&id).cloned() else {
            return Vec::new();
        };
        if publish_to_daemon {
            if let Some(tab) = self.tab(&address.tab_id) {
                publish_runtime_output_change(&tab.project_root, &id, &data);
            }
        }
        vec![OutboundEvent::broadcast(BackendEvent::TerminalOutput {
            id,
            data_base64: base64::engine::general_purpose::STANDARD.encode(data),
        })]
    }

    pub(crate) fn handle_runtime_status(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_status_inner(id, status, detail, true)
    }

    pub(crate) fn handle_daemon_runtime_status(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_status_inner(id, status, detail, false)
    }

    fn handle_runtime_status_inner(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
        publish_to_daemon: bool,
    ) -> Vec<OutboundEvent> {
        let Some(_address) = self.window_lookup.get(&id).cloned() else {
            self.remove_window_state_tracking(&id);
            self.mark_agent_session_stopped(&id);
            self.deregister_pty_writer(&id);
            self.runtimes.remove(&id);
            self.window_details.remove(&id);
            // SPEC-3214 FR-002: the status arrived after the window was torn
            // down, so the PTY is gone — safe point to destroy any pending
            // intake worktree.
            return self.take_ephemeral_worktree_cleanup_events();
        };
        let is_agent_window = self.window_preset(&id) == Some(WindowPreset::Agent);
        if publish_to_daemon {
            if let Some(address) = self.window_lookup.get(&id) {
                if let Some(tab) = self.tab(&address.tab_id) {
                    publish_runtime_status_change(&tab.project_root, &id, status, detail.clone());
                }
            }
        }

        // SPEC #3200 T-045/FR-025: a running agent on a monitored autonomous
        // issue is a liveness signal — refresh its stuck-detection window.
        if is_agent_window && matches!(status, WindowProcessStatus::Running) {
            self.issue_monitor_heartbeat(&id);
        }

        let keep_active_agent_session_for_recovery =
            self.should_keep_active_agent_session_for_recoverable_pty_error(&id, status);
        if matches!(status, WindowProcessStatus::Error) {
            self.window_hook_states.remove(&id);
        }
        self.window_pty_statuses.insert(id.clone(), status);
        let composed_status = self.recompute_window_state(&id).unwrap_or(status);
        let should_auto_close =
            should_auto_close_agent_window(&self.active_agent_sessions, &id, &composed_status)
                && self.window_hook_states.get(&id).copied() == Some(WindowProcessStatus::Stopped);
        match detail.as_ref() {
            Some(detail) if !detail.is_empty() => {
                self.window_details.insert(id.clone(), detail.clone());
            }
            _ => {
                self.window_details.remove(&id);
            }
        }
        if should_auto_close {
            self.clear_agent_window_startup_restore(&id);
            self.stop_window_runtime(&id);
            self.remove_window_state_tracking(&id);
            // SPEC-3214 FR-002: `stop_window_runtime` above killed and joined
            // the PTY, so a pending intake worktree can be destroyed now.
            let cleanup_events = self.take_ephemeral_worktree_cleanup_events();
            if !close_window_from_workspace(
                &mut self.tabs,
                &mut self.window_lookup,
                &mut self.window_details,
                &id,
            ) {
                return cleanup_events;
            }
            let _ = self.persist();
            let mut events = cleanup_events;
            self.push_workspace_and_active_work_projection_broadcasts(&mut events);
            return events;
        }
        if keep_active_agent_session_for_recovery {
            self.recoverable_agent_error_windows.insert(id.clone());
        } else if status != WindowProcessStatus::Error {
            self.recoverable_agent_error_windows.remove(&id);
        }
        if matches!(
            status,
            WindowProcessStatus::Error | WindowProcessStatus::Stopped
        ) && !keep_active_agent_session_for_recovery
        {
            self.runtimes.remove(&id);
            self.remove_window_state_tracking(&id);
            self.mark_agent_session_stopped(&id);
        }
        let _ = self.persist();

        // SPEC-3214 FR-002: a Stopped/Error status means the PTY process has
        // exited — drain any intake worktree cleanup queued by the session
        // stop above (or by an earlier explicit stop of this window).
        let mut events = self.take_ephemeral_worktree_cleanup_events();
        if is_agent_window
            && composed_status == WindowProcessStatus::Error
            && !keep_active_agent_session_for_recovery
        {
            let message = detail
                .as_deref()
                .unwrap_or("Agent entered error state")
                .to_string();
            events.extend(self.issue_monitor_agent_failed_events(&id, &message));
        }
        if matches!(
            status,
            WindowProcessStatus::Error | WindowProcessStatus::Stopped
        ) {
            if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
                events.push(event);
            }
        }
        events.extend(Self::status_events(id, composed_status, detail));
        events
    }

    pub(crate) fn handle_runtime_hook_event(
        &mut self,
        event: gwt::RuntimeHookEvent,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_hook_event_inner(event, true)
    }

    pub(crate) fn handle_daemon_runtime_hook_event(
        &mut self,
        event: gwt::RuntimeHookEvent,
    ) -> Vec<OutboundEvent> {
        self.handle_runtime_hook_event_inner(event, false)
    }

    fn handle_runtime_hook_event_inner(
        &mut self,
        event: gwt::RuntimeHookEvent,
        publish_to_daemon: bool,
    ) -> Vec<OutboundEvent> {
        if publish_to_daemon {
            if let Some(project_root) = event.project_root.as_deref().map(PathBuf::from) {
                publish_runtime_hook_change(&project_root, &event);
            }
        }
        let mut events = Vec::new();
        if Self::should_broadcast_runtime_hook_event_to_frontend(&event) {
            events.push(OutboundEvent::broadcast(BackendEvent::RuntimeHookEvent {
                event: event.clone(),
            }));
        }
        let Some(window_id) = self.active_window_for_runtime_event(&event) else {
            return events;
        };
        let is_agent_window = self.window_preset(&window_id) == Some(WindowPreset::Agent);
        let Some(hook_state) = gwt::window_state::runtime_hook_window_state(&event) else {
            return events;
        };
        self.recoverable_agent_error_windows.remove(&window_id);
        if self.window_hook_states.get(&window_id).copied() == Some(hook_state) {
            return events;
        }
        self.window_hook_states
            .insert(window_id.clone(), hook_state);
        let Some(composed_state) = self.recompute_window_state(&window_id) else {
            return events;
        };
        let hook_detail = event
            .message
            .as_deref()
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(str::to_string);
        let should_auto_close = should_auto_close_agent_window(
            &self.active_agent_sessions,
            &window_id,
            &composed_state,
        );
        if should_auto_close {
            self.clear_agent_window_startup_restore(&window_id);
            self.stop_window_runtime(&window_id);
            self.remove_window_state_tracking(&window_id);
            // SPEC-3214 FR-002: PTY killed and joined above — safe to destroy
            // a pending intake worktree.
            events.extend(self.take_ephemeral_worktree_cleanup_events());
            if close_window_from_workspace(
                &mut self.tabs,
                &mut self.window_lookup,
                &mut self.window_details,
                &window_id,
            ) {
                let _ = self.persist();
                self.push_workspace_and_active_work_projection_broadcasts(&mut events);
            }
            return events;
        }
        if gwt::window_state::is_live_agent_hook_state(hook_state) {
            self.window_details.remove(&window_id);
        } else if let Some(detail) = hook_detail.as_ref() {
            self.window_details
                .insert(window_id.clone(), detail.clone());
        }
        let detail = hook_detail.or_else(|| self.window_details.get(&window_id).cloned());
        let _ = self.persist();
        if is_agent_window && composed_state == WindowProcessStatus::Error {
            let message = detail
                .as_deref()
                .unwrap_or("Agent entered error state")
                .to_string();
            events.extend(self.issue_monitor_agent_failed_events(&window_id, &message));
        }
        if matches!(
            composed_state,
            WindowProcessStatus::Error | WindowProcessStatus::Stopped
        ) {
            if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
                events.push(event);
            }
        }
        events.extend(Self::status_events(window_id, composed_state, detail));
        events
    }

    fn should_keep_active_agent_session_for_recoverable_pty_error(
        &self,
        window_id: &str,
        status: WindowProcessStatus,
    ) -> bool {
        status == WindowProcessStatus::Error
            && self.active_agent_sessions.contains_key(window_id)
            && self
                .window_preset(window_id)
                .is_some_and(gwt::window_state::uses_agent_hook_state)
            && (self
                .window_hook_states
                .get(window_id)
                .is_some_and(|state| gwt::window_state::is_live_agent_hook_state(*state))
                || self.recoverable_agent_error_windows.contains(window_id))
    }

    fn should_broadcast_runtime_hook_event_to_frontend(event: &gwt::RuntimeHookEvent) -> bool {
        event.kind != gwt::RuntimeHookEventKind::RuntimeState
    }
}

#[cfg(unix)]
const RUNTIME_DAEMON_PUBLISH_QUEUE_CAPACITY: usize = 4096;

#[cfg(unix)]
enum RuntimeDaemonPublish {
    Output {
        project_root: PathBuf,
        id: String,
        data: Vec<u8>,
    },
    Status {
        project_root: PathBuf,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    },
    Hook {
        project_root: PathBuf,
        event: gwt::RuntimeHookEvent,
    },
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeDaemonPublishEnqueueError {
    Full,
    Disconnected,
}

#[cfg(unix)]
static RUNTIME_DAEMON_PUBLISH_QUEUE: std::sync::OnceLock<
    Mutex<Option<std_mpsc::SyncSender<RuntimeDaemonPublish>>>,
> = std::sync::OnceLock::new();

#[cfg(unix)]
fn runtime_daemon_publish_sender() -> Option<std_mpsc::SyncSender<RuntimeDaemonPublish>> {
    let queue = RUNTIME_DAEMON_PUBLISH_QUEUE.get_or_init(|| Mutex::new(None));
    runtime_daemon_publish_sender_from(queue, |receiver| {
        std::thread::Builder::new()
            .name("gwt-runtime-daemon-publish-worker".to_string())
            .spawn(move || run_runtime_daemon_publish_worker(receiver))
            .map(|_handle| ())
    })
}

#[cfg(unix)]
fn runtime_daemon_publish_sender_from(
    queue: &Mutex<Option<std_mpsc::SyncSender<RuntimeDaemonPublish>>>,
    spawn_worker: impl FnOnce(std_mpsc::Receiver<RuntimeDaemonPublish>) -> std::io::Result<()>,
) -> Option<std_mpsc::SyncSender<RuntimeDaemonPublish>> {
    let Ok(mut queue) = queue.lock() else {
        tracing::debug!("runtime daemon publish queue lock poisoned");
        return None;
    };
    if let Some(sender) = queue.as_ref() {
        return Some(sender.clone());
    }

    let (sender, receiver) = std_mpsc::sync_channel(RUNTIME_DAEMON_PUBLISH_QUEUE_CAPACITY);
    match spawn_worker(receiver) {
        Ok(()) => {
            *queue = Some(sender.clone());
            Some(sender)
        }
        Err(err) => {
            tracing::debug!(error = %err, "runtime daemon publish worker spawn failed");
            None
        }
    }
}

#[cfg(unix)]
fn run_runtime_daemon_publish_worker(receiver: std_mpsc::Receiver<RuntimeDaemonPublish>) {
    for publish in receiver {
        publish_runtime_daemon_event(publish);
    }
}

#[cfg(unix)]
fn try_enqueue_runtime_daemon_publish(
    sender: &std_mpsc::SyncSender<RuntimeDaemonPublish>,
    publish: RuntimeDaemonPublish,
) -> Result<(), RuntimeDaemonPublishEnqueueError> {
    sender.try_send(publish).map_err(|err| match err {
        std_mpsc::TrySendError::Full(_) => RuntimeDaemonPublishEnqueueError::Full,
        std_mpsc::TrySendError::Disconnected(_) => RuntimeDaemonPublishEnqueueError::Disconnected,
    })
}

#[cfg(unix)]
fn enqueue_runtime_daemon_publish(publish: RuntimeDaemonPublish) {
    let Some(sender) = runtime_daemon_publish_sender() else {
        return;
    };
    if let Err(err) = try_enqueue_runtime_daemon_publish(&sender, publish) {
        tracing::debug!(
            ?err,
            "runtime daemon publish queue rejected event (non-fatal)"
        );
    }
}

#[cfg(unix)]
fn publish_runtime_daemon_event(publish: RuntimeDaemonPublish) {
    match publish {
        RuntimeDaemonPublish::Output {
            project_root,
            id,
            data,
        } => {
            let payload =
                gwt::runtime_daemon_events::runtime_output_payload(&id, &data, std::process::id());
            let result = gwt::daemon_publisher::publish_event(
                &project_root,
                gwt::runtime_daemon_events::RUNTIME_OUTPUT_CHANNEL,
                payload,
            );
            if let Err(err) = result {
                tracing::debug!(
                    error = %err,
                    project_root = %project_root.display(),
                    window_id = %id,
                    "runtime output daemon publish failed (non-fatal)"
                );
            }
        }
        RuntimeDaemonPublish::Status {
            project_root,
            id,
            status,
            detail,
        } => {
            let payload = gwt::runtime_daemon_events::runtime_status_payload(
                &id,
                status,
                detail,
                std::process::id(),
            );
            let result = gwt::daemon_publisher::publish_event(
                &project_root,
                gwt::runtime_daemon_events::RUNTIME_STATUS_CHANNEL,
                payload,
            );
            if let Err(err) = result {
                tracing::debug!(
                    error = %err,
                    project_root = %project_root.display(),
                    window_id = %id,
                    "runtime status daemon publish failed (non-fatal)"
                );
            }
        }
        RuntimeDaemonPublish::Hook {
            project_root,
            event,
        } => {
            let payload =
                gwt::runtime_daemon_events::runtime_hook_payload(&event, std::process::id());
            let result = gwt::daemon_publisher::publish_event(
                &project_root,
                gwt::runtime_daemon_events::RUNTIME_HOOK_CHANNEL,
                payload,
            );
            if let Err(err) = result {
                tracing::debug!(
                    error = %err,
                    project_root = %project_root.display(),
                    "runtime hook daemon publish failed (non-fatal)"
                );
            }
        }
    }
}

#[cfg(unix)]
fn publish_runtime_output_change(project_root: &Path, id: &str, data: &[u8]) {
    enqueue_runtime_daemon_publish(RuntimeDaemonPublish::Output {
        project_root: project_root.to_path_buf(),
        id: id.to_string(),
        data: data.to_vec(),
    });
}

#[cfg(not(unix))]
fn publish_runtime_output_change(_project_root: &Path, _id: &str, _data: &[u8]) {}

#[cfg(unix)]
fn publish_runtime_status_change(
    project_root: &Path,
    id: &str,
    status: WindowProcessStatus,
    detail: Option<String>,
) {
    enqueue_runtime_daemon_publish(RuntimeDaemonPublish::Status {
        project_root: project_root.to_path_buf(),
        id: id.to_string(),
        status,
        detail,
    });
}

#[cfg(not(unix))]
fn publish_runtime_status_change(
    _project_root: &Path,
    _id: &str,
    _status: WindowProcessStatus,
    _detail: Option<String>,
) {
}

#[cfg(unix)]
fn publish_runtime_hook_change(project_root: &Path, event: &gwt::RuntimeHookEvent) {
    enqueue_runtime_daemon_publish(RuntimeDaemonPublish::Hook {
        project_root: project_root.to_path_buf(),
        event: event.clone(),
    });
}

#[cfg(not(unix))]
fn publish_runtime_hook_change(_project_root: &Path, _event: &gwt::RuntimeHookEvent) {}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::PathBuf;

    #[cfg(unix)]
    use std::sync::{mpsc, Mutex};

    #[cfg(unix)]
    use super::{
        runtime_daemon_publish_sender_from, try_enqueue_runtime_daemon_publish,
        RuntimeDaemonPublish, RuntimeDaemonPublishEnqueueError,
    };
    #[cfg(unix)]
    use crate::WindowProcessStatus;

    #[cfg(unix)]
    #[test]
    fn runtime_daemon_publish_enqueue_is_bounded_and_nonblocking() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let project_root = PathBuf::from("/tmp/gwt-project");

        assert!(try_enqueue_runtime_daemon_publish(
            &sender,
            RuntimeDaemonPublish::Output {
                project_root: project_root.clone(),
                id: "tab-1::shell-1".to_string(),
                data: b"first".to_vec(),
            },
        )
        .is_ok());
        assert!(matches!(
            try_enqueue_runtime_daemon_publish(
                &sender,
                RuntimeDaemonPublish::Status {
                    project_root,
                    id: "tab-1::shell-1".to_string(),
                    status: WindowProcessStatus::Running,
                    detail: None,
                },
            ),
            Err(RuntimeDaemonPublishEnqueueError::Full)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_daemon_publish_sender_retries_after_spawn_failure() {
        let queue = Mutex::new(None);

        assert!(runtime_daemon_publish_sender_from(&queue, |_receiver| {
            Err(std::io::Error::other("spawn failed"))
        })
        .is_none());
        assert!(queue.lock().expect("queue").is_none());

        assert!(runtime_daemon_publish_sender_from(&queue, |_receiver| Ok(())).is_some());
        assert!(queue.lock().expect("queue").is_some());

        assert!(runtime_daemon_publish_sender_from(&queue, |_receiver| {
            panic!("sender should be reused without spawning a second worker")
        })
        .is_some());
    }
}
