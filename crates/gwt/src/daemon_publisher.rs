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
//!    persisted [`gwt_core::daemon::DaemonEndpoint`].
//! 2. If no live daemon is registered, returns
//!    `Err("daemon not running")` so the caller can continue with the
//!    local file path as the source of truth.
//! 3. Otherwise opens a single-shot [`DaemonClient`] connection,
//!    sends one [`ClientFrame::Publish`], and waits for the daemon's
//!    `Ack`.
//!
//! Connect, publish, and ack are each bounded by `timeout` (default
//! 200 ms per stage, so the worst-case wall time is ~600 ms across
//! the three stages). Generic notifications remain best-effort. Issue Monitor
//! controls use a stricter typed path: after a live endpoint establishes daemon
//! authority, connection/send/receipt uncertainty never authorizes a local
//! fallback writer.

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
use crate::runtime_daemon_events::{
    IssueMonitorControlPublishError, ISSUE_MONITOR_CONTROL_BUSY_ERROR,
    ISSUE_MONITOR_CONTROL_CHANNEL, ISSUE_MONITOR_CONTROL_RECOVERY_BLOCKED_ERROR,
};

/// Default per-stage timeout for the GUI / CLI hot path. 200 ms is
/// generous for a local Unix-socket round-trip (typical is < 5 ms) but
/// short enough that a hung daemon cannot freeze the caller for more
/// than 600 ms total (connect + send + ack — three independent
/// stages, see [`publish_event_with_timeout`]). Phase H1 GREEN handler
/// integration trades the small worst-case stall for code-path
/// simplicity. Callers needing a different budget should use
/// [`publish_event_with_timeout`].
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(200);

/// Publish `payload` to `channel` on the daemon for `project_root`.
///
/// Default per-stage timeout (200 ms) bounds connect, send, and ack
/// independently, so total wall time is at most 600 ms even when the
/// daemon is hung. See [`publish_event_with_timeout`] when callers
/// need a custom budget.
pub fn publish_event(project_root: &Path, channel: &str, payload: Value) -> Result<(), String> {
    publish_event_with_timeout(project_root, channel, payload, DEFAULT_TIMEOUT)
}

/// Publish an Issue Monitor command with an outcome classification that lets
/// the GUI distinguish a definitely-unsent transport failure from an
/// ambiguous or explicitly rejected command. Only the former may use the
/// local single-writer fallback.
pub fn publish_issue_monitor_control(
    project_root: &Path,
    payload: Value,
) -> Result<(), IssueMonitorControlPublishError> {
    publish_issue_monitor_control_with_timeout(project_root, payload, DEFAULT_TIMEOUT)
}

fn publish_issue_monitor_control_with_timeout(
    project_root: &Path,
    payload: Value,
    timeout: Duration,
) -> Result<(), IssueMonitorControlPublishError> {
    publish_issue_monitor_control_with_timeout_and_liveness(
        project_root,
        payload,
        timeout,
        is_alive,
    )
}

#[derive(Debug)]
enum EndpointAbsenceEvidence {
    Missing,
    DefinitelyDead,
    Uncertain(String),
}

fn authority_fence_allows_fallback(project_root: &Path) -> Result<(), String> {
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(project_root);
    match crate::load_issue_monitor_authority_fence(&prefs_path) {
        Ok(crate::IssueMonitorAuthorityFenceState::Missing) => Ok(()),
        Ok(crate::IssueMonitorAuthorityFenceState::LegacyShutdownRevoke) => {
            Err("Issue Monitor authority fence retains pending crash recovery".to_string())
        }
        Ok(crate::IssueMonitorAuthorityFenceState::Active(fence)) => Err(format!(
            "Issue Monitor authority fence is owned by daemon pid {}",
            fence.pid
        )),
        Err(error) => Err(format!(
            "Issue Monitor authority fence is malformed or unreadable: {error}"
        )),
    }
}

fn endpoint_absence_evidence(
    endpoint_path: &Path,
    is_process_alive: &impl Fn(u32) -> bool,
) -> EndpointAbsenceEvidence {
    let payload = match std::fs::read(endpoint_path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return EndpointAbsenceEvidence::Missing;
        }
        Err(error) => {
            return EndpointAbsenceEvidence::Uncertain(format!(
                "endpoint evidence is unreadable: {error}"
            ));
        }
    };
    let endpoint = match serde_json::from_slice::<gwt_core::daemon::DaemonEndpoint>(&payload) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            return EndpointAbsenceEvidence::Uncertain(format!(
                "endpoint evidence is malformed: {error}"
            ));
        }
    };
    if endpoint.pid > 0 && !is_process_alive(endpoint.pid) {
        EndpointAbsenceEvidence::DefinitelyDead
    } else {
        EndpointAbsenceEvidence::Uncertain(format!(
            "live endpoint mismatch or invalid endpoint metadata for pid {}",
            endpoint.pid
        ))
    }
}

fn publish_issue_monitor_control_with_timeout_and_liveness(
    project_root: &Path,
    payload: Value,
    timeout: Duration,
    is_process_alive: impl Fn(u32) -> bool,
) -> Result<(), IssueMonitorControlPublishError> {
    use IssueMonitorControlPublishError::{
        Busy, OutcomeUnknown, RecoveryBlocked, Rejected, TransportUnavailable,
    };

    let started = std::time::Instant::now();
    let scope = RuntimeScope::from_project_root(project_root, RuntimeTarget::Host)
        .map_err(|error| OutcomeUnknown(format!("scope resolution failed: {error}")))?;
    let gwt_home = paths::gwt_home();
    let endpoint_path = scope.endpoint_path(&gwt_home);
    let absence_evidence = endpoint_absence_evidence(&endpoint_path, &is_process_alive);
    let fence_evidence = authority_fence_allows_fallback(project_root);
    let action = resolve_bootstrap_action(&gwt_home, &scope, DAEMON_PROTOCOL_VERSION, |pid| {
        is_process_alive(pid)
    })
    .map_err(|error| OutcomeUnknown(format!("bootstrap resolve failed: {error}")))?;
    let remaining_budget = || -> Result<Duration, IssueMonitorControlPublishError> {
        timeout
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                OutcomeUnknown(format!(
                    "publish budget exhausted during scope/bootstrap resolution after {}ms",
                    timeout.as_millis()
                ))
            })
    };
    remaining_budget()?;
    let endpoint = match action {
        DaemonBootstrapAction::Reuse(endpoint) => endpoint,
        DaemonBootstrapAction::Spawn { .. } => match absence_evidence {
            EndpointAbsenceEvidence::Missing | EndpointAbsenceEvidence::DefinitelyDead => {
                match fence_evidence {
                    Ok(()) => {
                        return Err(TransportUnavailable("daemon not running".to_string()));
                    }
                    Err(reason) => return Err(OutcomeUnknown(reason)),
                }
            }
            EndpointAbsenceEvidence::Uncertain(reason) => return Err(OutcomeUnknown(reason)),
        },
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| OutcomeUnknown(format!("tokio runtime build failed: {error}")))?;
    let remaining = remaining_budget()?;
    runtime.block_on(async move {
        let deadline = tokio::time::Instant::now() + remaining;
        let mut busy_backoff = Duration::from_millis(5);
        loop {
            let mut client = tokio::time::timeout_at(deadline, DaemonClient::connect(&endpoint))
                .await
                .map_err(|_| {
                    OutcomeUnknown(format!("connect timeout after {}ms", timeout.as_millis()))
                })?
                .map_err(OutcomeUnknown)?;
            let publish_frame = ClientFrame::Publish {
                channel: ISSUE_MONITOR_CONTROL_CHANNEL.to_string(),
                payload: payload.clone(),
            };
            tokio::time::timeout_at(deadline, client.send_frame(&publish_frame))
                .await
                .map_err(|_| {
                    OutcomeUnknown(format!(
                        "publish send timeout after {}ms",
                        timeout.as_millis()
                    ))
                })?
                .map_err(|error| OutcomeUnknown(format!("publish send failed: {error}")))?;
            let response: DaemonFrame = tokio::time::timeout_at(deadline, client.read_frame())
                .await
                .map_err(|_| {
                    OutcomeUnknown(format!(
                        "publish ack timeout after {}ms",
                        timeout.as_millis()
                    ))
                })?
                .map_err(OutcomeUnknown)?;
            match response {
                DaemonFrame::Ack => return Ok(()),
                DaemonFrame::Error { message }
                    if message == ISSUE_MONITOR_CONTROL_RECOVERY_BLOCKED_ERROR =>
                {
                    return Err(RecoveryBlocked);
                }
                DaemonFrame::Error { message } if message == ISSUE_MONITOR_CONTROL_BUSY_ERROR => {
                    let now = tokio::time::Instant::now();
                    let Some(remaining) = deadline.checked_duration_since(now) else {
                        return Err(Busy(message));
                    };
                    if remaining.is_zero() {
                        return Err(Busy(message));
                    }
                    tokio::time::sleep(remaining.min(busy_backoff)).await;
                    if tokio::time::Instant::now() >= deadline {
                        return Err(Busy(message));
                    }
                    busy_backoff = busy_backoff
                        .saturating_mul(2)
                        .min(Duration::from_millis(50));
                }
                DaemonFrame::Error { message } => return Err(Rejected(message)),
                other => {
                    return Err(OutcomeUnknown(format!("expected Ack, got: {other:?}")));
                }
            }
        }
    })
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

// Liveness probe shared with `cli::daemon` and `main`; see
// `crate::process::is_process_alive`.
use crate::process::is_process_alive as is_alive;

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, Write},
        os::unix::net::UnixListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        time::Duration,
    };

    use gwt_core::daemon::{
        persist_endpoint, ClientFrame, DaemonEndpoint, DaemonFrame, IpcHandshakeRequest,
        IpcHandshakeResponse, RuntimeScope, RuntimeTarget, DAEMON_PROTOCOL_VERSION,
    };
    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        publish_event_with_timeout, publish_issue_monitor_control_with_timeout,
        publish_issue_monitor_control_with_timeout_and_liveness,
    };

    #[test]
    fn publish_returns_error_when_no_daemon_registered() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    #[test]
    fn live_endpoint_connect_failure_is_outcome_unknown_not_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
            .expect("runtime scope");
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            project
                .path()
                .join("refused.sock")
                .to_string_lossy()
                .to_string(),
            "test-token".to_string(),
            "test-daemon".to_string(),
        );
        persist_endpoint(
            &scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &endpoint,
        )
        .expect("persist live endpoint");

        let error = publish_issue_monitor_control_with_timeout(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(100),
        )
        .expect_err("live endpoint connection is refused");

        assert!(
            !error.allows_local_fallback(),
            "Reuse establishes daemon authority even when this connection fails: {error}"
        );
        assert!(matches!(
            error,
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(_)
        ));
    }

    #[test]
    fn scope_resolution_failure_is_outcome_unknown_not_fallback() {
        let error = publish_issue_monitor_control_with_timeout(
            Path::new("relative-project-root"),
            json!({"enabled": false}),
            Duration::from_millis(100),
        )
        .expect_err("relative project root cannot establish daemon absence");

        assert!(!error.allows_local_fallback());
        assert!(matches!(
            error,
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(message)
                if message.contains("scope resolution failed")
        ));
    }

    #[test]
    fn bootstrap_resolution_failure_is_outcome_unknown_not_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
            .expect("runtime scope");
        std::fs::create_dir_all(scope.endpoint_path(&gwt_core::paths::gwt_home()))
            .expect("replace endpoint file with a directory");

        let error = publish_issue_monitor_control_with_timeout(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(100),
        )
        .expect_err("unreadable endpoint cannot establish daemon absence");

        assert!(!error.allows_local_fallback());
        assert!(matches!(
            error,
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(message)
                if message.contains("bootstrap resolve failed")
        ));
    }

    #[test]
    fn malformed_endpoint_spawn_is_outcome_unknown_not_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
            .expect("runtime scope");
        let endpoint_path = scope.endpoint_path(&gwt_core::paths::gwt_home());
        std::fs::create_dir_all(endpoint_path.parent().expect("endpoint parent"))
            .expect("create endpoint parent");
        std::fs::write(&endpoint_path, b"{\"pid\":").expect("seed malformed endpoint");

        let error = publish_issue_monitor_control_with_timeout(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(100),
        )
        .expect_err("malformed endpoint cannot prove daemon absence");

        assert!(!error.allows_local_fallback());
        assert!(matches!(
            error,
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(message)
                if message.contains("endpoint evidence")
        ));
    }

    #[test]
    fn live_protocol_mismatch_spawn_is_outcome_unknown_not_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
            .expect("runtime scope");
        let mut endpoint = DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            home.path()
                .join("mismatch.sock")
                .to_string_lossy()
                .to_string(),
            "mismatch-token".to_string(),
            "test-daemon".to_string(),
        );
        endpoint.protocol_version = DAEMON_PROTOCOL_VERSION.wrapping_add(1);
        persist_endpoint(
            &scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &endpoint,
        )
        .expect("persist live mismatched endpoint");

        let error = publish_issue_monitor_control_with_timeout(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(100),
        )
        .expect_err("live mismatch cannot prove daemon absence");

        assert!(!error.allows_local_fallback());
        assert!(matches!(
            error,
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(message)
                if message.contains("live endpoint mismatch")
        ));
    }

    #[test]
    fn live_scope_mismatch_spawn_is_outcome_unknown_not_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let other_project = TempDir::new().expect("other project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
            .expect("runtime scope");
        let other_scope =
            RuntimeScope::from_project_root(other_project.path(), RuntimeTarget::Host)
                .expect("other runtime scope");
        let endpoint = DaemonEndpoint::new(
            other_scope,
            std::process::id(),
            home.path()
                .join("scope-mismatch.sock")
                .to_string_lossy()
                .to_string(),
            "scope-mismatch-token".to_string(),
            "test-daemon".to_string(),
        );
        persist_endpoint(
            &scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &endpoint,
        )
        .expect("persist live scope-mismatched endpoint");

        let error = publish_issue_monitor_control_with_timeout(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(100),
        )
        .expect_err("live scope mismatch cannot prove daemon absence");

        assert!(!error.allows_local_fallback());
        assert!(matches!(
            error,
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(message)
                if message.contains("live endpoint mismatch")
        ));
    }

    #[test]
    fn missing_or_definitely_dead_endpoint_allows_local_fallback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());

        let missing = publish_issue_monitor_control_with_timeout_and_liveness(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(100),
            |_| false,
        )
        .expect_err("missing endpoint uses local fallback");
        assert!(missing.allows_local_fallback());

        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
            .expect("runtime scope");
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            424_242,
            home.path().join("dead.sock").to_string_lossy().to_string(),
            "dead-token".to_string(),
            "test-daemon".to_string(),
        );
        persist_endpoint(
            &scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &endpoint,
        )
        .expect("persist dead endpoint");
        let dead = publish_issue_monitor_control_with_timeout_and_liveness(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(100),
            |_| false,
        )
        .expect_err("definitely dead endpoint uses local fallback");
        assert!(dead.allows_local_fallback());
    }

    #[test]
    fn missing_endpoint_with_active_authority_fence_is_outcome_unknown() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(project.path());
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default())
            .expect("seed prefs");
        crate::persist_issue_monitor_authority_fence(
            &prefs_path,
            &crate::IssueMonitorAuthorityFence::current_process(),
        )
        .expect("persist active authority fence");

        let error = publish_issue_monitor_control_with_timeout_and_liveness(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(100),
            |_| false,
        )
        .expect_err("active fence prevents missing-endpoint fallback");

        assert!(!error.allows_local_fallback());
        assert!(matches!(
            error,
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(message)
                if message.contains("authority fence")
        ));
    }

    #[test]
    fn missing_endpoint_with_malformed_authority_fence_is_outcome_unknown_and_retained() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(project.path());
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default())
            .expect("seed prefs");
        let fence_path = crate::issue_monitor_authority_fence_path(&prefs_path);
        let malformed = b"{\"version\":1,";
        std::fs::write(&fence_path, malformed).expect("seed malformed authority fence");

        let error = publish_issue_monitor_control_with_timeout_and_liveness(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(100),
            |_| false,
        )
        .expect_err("ambiguous fence prevents missing-endpoint fallback");

        assert!(!error.allows_local_fallback());
        assert!(matches!(
            error,
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(message)
                if message.contains("malformed or unreadable")
        ));
        assert_eq!(
            std::fs::read(fence_path).expect("reload malformed fence"),
            malformed
        );
    }

    #[test]
    fn bootstrap_elapsed_time_consumes_the_control_budget() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
            .expect("runtime scope");
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            424_242,
            home.path()
                .join("budget-exhausted.sock")
                .to_string_lossy()
                .to_string(),
            "budget-token".to_string(),
            "test-daemon".to_string(),
        );
        persist_endpoint(
            &scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &endpoint,
        )
        .expect("persist live endpoint fixture");

        let error = publish_issue_monitor_control_with_timeout_and_liveness(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(5),
            |_| {
                std::thread::sleep(Duration::from_millis(10));
                true
            },
        )
        .expect_err("bootstrap work consumes the absolute publish budget");

        assert!(!error.allows_local_fallback());
        assert!(matches!(
            error,
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(message)
                if message.contains("budget exhausted during scope/bootstrap")
        ));
    }

    #[test]
    fn busy_control_is_retried_with_the_same_payload_until_committed_ack() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
            .expect("runtime scope");
        let socket_path = home.path().join("busy-retry.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind test daemon");
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            socket_path.to_string_lossy().to_string(),
            "busy-retry-token".to_string(),
            "test-daemon".to_string(),
        );
        persist_endpoint(
            &scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &endpoint,
        )
        .expect("persist live endpoint");
        let received = Arc::new(Mutex::new(Vec::new()));
        let server_received = Arc::clone(&received);
        let server = std::thread::spawn(move || {
            for response in [
                DaemonFrame::Error {
                    message: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_BUSY_ERROR
                        .to_string(),
                },
                DaemonFrame::Ack,
            ] {
                let (stream, _) = listener.accept().expect("accept publisher");
                let mut reader =
                    std::io::BufReader::new(stream.try_clone().expect("clone publisher stream"));
                let mut writer = stream;
                let mut line = String::new();
                reader.read_line(&mut line).expect("read handshake");
                let request: IpcHandshakeRequest =
                    serde_json::from_str(line.trim_end()).expect("parse handshake");
                assert_eq!(request.scope, scope);
                let handshake = IpcHandshakeResponse {
                    protocol_version: DAEMON_PROTOCOL_VERSION,
                    daemon_version: "test-daemon".to_string(),
                    accepted: true,
                    rejection_reason: None,
                };
                writeln!(
                    writer,
                    "{}",
                    serde_json::to_string(&handshake).expect("serialize handshake")
                )
                .expect("write handshake");
                line.clear();
                reader.read_line(&mut line).expect("read publish");
                let frame: ClientFrame =
                    serde_json::from_str(line.trim_end()).expect("parse publish");
                let ClientFrame::Publish { payload, .. } = frame else {
                    panic!("expected control publish");
                };
                server_received
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(payload);
                writeln!(
                    writer,
                    "{}",
                    serde_json::to_string(&response).expect("serialize response")
                )
                .expect("write response");
            }
        });
        let payload = json!({"enabled": false, "source": "busy-retry"});

        publish_issue_monitor_control_with_timeout(
            project.path(),
            payload.clone(),
            Duration::from_millis(500),
        )
        .expect("explicit Busy is safely retried");
        server.join().expect("test daemon joins");

        assert_eq!(
            *received
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            vec![payload.clone(), payload]
        );
    }

    #[test]
    fn busy_control_budget_exhaustion_is_a_non_fallback_busy_error() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _home_guard = ScopedEnvVar::set("HOME", home.path());
        let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
            .expect("runtime scope");
        let socket_path = home.path().join("busy-exhausted.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind test daemon");
        listener
            .set_nonblocking(true)
            .expect("nonblocking test listener");
        let endpoint = DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            socket_path.to_string_lossy().to_string(),
            "busy-exhausted-token".to_string(),
            "test-daemon".to_string(),
        );
        persist_endpoint(
            &scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &endpoint,
        )
        .expect("persist live endpoint");
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let server = std::thread::spawn(move || {
            while !server_stop.load(Ordering::Acquire) {
                let (stream, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(error) => panic!("accept publisher: {error}"),
                };
                stream
                    .set_nonblocking(false)
                    .expect("blocking publisher stream");
                let mut reader =
                    std::io::BufReader::new(stream.try_clone().expect("clone publisher stream"));
                let mut writer = stream;
                let mut line = String::new();
                reader.read_line(&mut line).expect("read handshake");
                let request: IpcHandshakeRequest =
                    serde_json::from_str(line.trim_end()).expect("parse handshake");
                assert_eq!(request.scope, scope);
                let handshake = IpcHandshakeResponse {
                    protocol_version: DAEMON_PROTOCOL_VERSION,
                    daemon_version: "test-daemon".to_string(),
                    accepted: true,
                    rejection_reason: None,
                };
                writeln!(
                    writer,
                    "{}",
                    serde_json::to_string(&handshake).expect("serialize handshake")
                )
                .expect("write handshake");
                line.clear();
                reader.read_line(&mut line).expect("read publish");
                let _: ClientFrame = serde_json::from_str(line.trim_end()).expect("parse publish");
                let response = DaemonFrame::Error {
                    message: crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_BUSY_ERROR
                        .to_string(),
                };
                writeln!(
                    writer,
                    "{}",
                    serde_json::to_string(&response).expect("serialize response")
                )
                .expect("write response");
            }
        });

        let error = publish_issue_monitor_control_with_timeout(
            project.path(),
            json!({"enabled": false}),
            Duration::from_millis(40),
        )
        .expect_err("Busy must remain explicit when its retry budget expires");
        stop.store(true, Ordering::Release);
        server.join().expect("test daemon joins");

        assert!(!error.allows_local_fallback());
        assert!(matches!(
            error,
            crate::runtime_daemon_events::IssueMonitorControlPublishError::Busy(message)
                if message == crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_BUSY_ERROR
        ));
    }

    /// Minimal scoped env-var helper used by tests in this module to
    /// avoid pulling in the workspace-wide test_support graph.
    use gwt_core::test_support::ScopedEnvVar;
    use std::path::Path;
}
