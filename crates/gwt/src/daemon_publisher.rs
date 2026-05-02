//! Best-effort synchronous daemon publisher for SPEC-2077 Phase H1+.
//!
//! `publish_event` is the sync convenience wrapper that gwt-side
//! domain handlers (Board projection writer, runtime status emitter,
//! launch lifecycle hooks) call after a state change to fan the change
//! out across other gwt instances connected to the same daemon.
//!
//! The function:
//!
//! 1. Resolves the [`RuntimeScope`] for `project_root` and reads the
//!    persisted [`DaemonEndpoint`].
//! 2. If no live daemon is registered, returns
//!    `Err("daemon not running")` so the caller can continue with the
//!    local file path as the source of truth.
//! 3. Otherwise opens a single-shot [`DaemonClient`] connection,
//!    sends one [`ClientFrame::Publish`], and waits for the daemon's
//!    `Ack`.
//!
//! Connect, publish, and ack are each bounded by `timeout` (default
//! 2 s). On any error the caller should treat the publish as
//! best-effort — the local store remains authoritative.

#![cfg(unix)]

use std::{path::Path, time::Duration};

use gwt_core::{
    daemon::{
        resolve_bootstrap_action, ClientFrame, DaemonBootstrapAction, DaemonFrame, RuntimeScope,
        RuntimeTarget, DAEMON_PROTOCOL_VERSION,
    },
    paths,
};
use serde_json::Value;

use crate::cli::daemon::client::DaemonClient;

/// Default per-stage timeout for the GUI / CLI hot path. 200 ms is
/// generous for a local Unix-socket round-trip (typical is < 5 ms) but
/// short enough that a hung daemon cannot freeze the UI for more than
/// 400 ms total (connect + ack). Phase H1 GREEN handler integration
/// trades the small worst-case stall for code-path simplicity.
/// Callers needing a different budget should use
/// [`publish_event_with_timeout`].
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);

/// Publish `payload` to `channel` on the daemon for `project_root`.
///
/// Default per-stage timeout (200 ms) bounds connect and ack
/// independently, so total wall time is at most 400 ms even when the
/// daemon is hung. See [`publish_event_with_timeout`] when callers
/// need a custom budget.
pub fn publish_event(project_root: &Path, channel: &str, payload: Value) -> Result<(), String> {
    publish_event_with_timeout(project_root, channel, payload, DEFAULT_TIMEOUT)
}

/// Same as [`publish_event`] but lets the caller override the
/// connect / read / ack timeout budget. Each individual stage is
/// bounded by `timeout` so a stuck daemon cannot pin the calling
/// thread for longer than `2 * timeout` in the worst case.
pub fn publish_event_with_timeout(
    project_root: &Path,
    channel: &str,
    payload: Value,
    timeout: Duration,
) -> Result<(), String> {
    let scope = RuntimeScope::from_project_root(project_root, RuntimeTarget::Host)
        .map_err(|err| format!("scope resolution failed: {err}"))?;
    let gwt_home = paths::gwt_home();
    let action = resolve_bootstrap_action(&gwt_home, &scope, DAEMON_PROTOCOL_VERSION, is_alive)
        .map_err(|err| format!("bootstrap resolve failed: {err}"))?;
    let endpoint = match action {
        DaemonBootstrapAction::Reuse(ep) => ep,
        DaemonBootstrapAction::Spawn { .. } => return Err("daemon not running".to_string()),
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("tokio runtime build failed: {err}"))?;
    runtime.block_on(async move {
        let mut client = tokio::time::timeout(timeout, DaemonClient::connect(&endpoint))
            .await
            .map_err(|_| format!("connect timeout after {}ms", timeout.as_millis()))??;
        // Bound the send half too: a daemon that has accepted the
        // connection but stopped reading (or a payload large enough
        // to fill the socket buffer) can otherwise block the writer
        // forever, freezing the synchronous caller despite the
        // documented per-stage `timeout`.
        let publish_frame = ClientFrame::Publish {
            channel: channel.to_string(),
            payload,
        };
        tokio::time::timeout(timeout, client.send_frame(&publish_frame))
            .await
            .map_err(|_| format!("publish send timeout after {}ms", timeout.as_millis()))?
            .map_err(|err| format!("publish send failed: {err}"))?;
        let ack: DaemonFrame = tokio::time::timeout(timeout, client.read_frame())
            .await
            .map_err(|_| format!("publish ack timeout after {}ms", timeout.as_millis()))??;
        match ack {
            DaemonFrame::Ack => Ok(()),
            DaemonFrame::Error { message } => Err(format!("daemon rejected publish: {message}")),
            other => Err(format!("expected Ack, got: {other:?}")),
        }
    })
}

fn is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: kill(pid, 0) only probes the process; we never deliver a
    // real signal. Returns 0 if alive, -1 with ESRCH otherwise.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use tempfile::TempDir;

    use super::publish_event_with_timeout;

    #[test]
    fn publish_returns_error_when_no_daemon_registered() {
        // Use a tempdir for the project root and a tempdir for $HOME so
        // the resolver looks for the endpoint inside an empty
        // `~/.gwt/projects/.../runtime/daemon/` tree and finds nothing.
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        std::fs::create_dir_all(project.path()).expect("project dir");

        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());

        let err = publish_event_with_timeout(
            project.path(),
            "board",
            json!({"entries": 1}),
            Duration::from_millis(200),
        )
        .expect_err("expected error when no daemon is running");
        assert!(
            err.contains("daemon not running"),
            "unexpected error message: {err}"
        );
    }

    /// Minimal scoped env-var helper used by tests in this module to
    /// avoid pulling in the workspace-wide test_support graph.
    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: impl AsRef<std::path::Path>) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value.as_ref());
            }
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            unsafe {
                match self.previous.take() {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }
}
