//! `gwtd daemon ...` family — long-running runtime daemon (SPEC-2077).
//!
//! - `mod.rs` (this file): argv parsing + dispatch + status reporting.
//! - `server.rs`: tokio-based IPC listener (Unix domain socket today;
//!   Windows named-pipe support is a follow-up).
//!
//! The contract layer (`gwt_core::daemon::*`) defines the on-disk endpoint
//! file, handshake protocol, and `DaemonBootstrapAction`. `Start` honours
//! that contract: if a usable endpoint already exists for the cwd
//! [`RuntimeScope`], we exit 0 with a "already running" notice; otherwise
//! we generate a fresh `auth_token`, persist a new [`DaemonEndpoint`], and
//! enter the listen loop.

#[cfg(unix)]
pub(crate) mod broadcast;
#[cfg(unix)]
pub mod client;
#[cfg(unix)]
pub(crate) mod server;

use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

use gwt_core::daemon::{
    resolve_bootstrap_action, DaemonBootstrapAction, DaemonStatus, RuntimeScope, RuntimeTarget,
    DAEMON_PROTOCOL_VERSION,
};
#[cfg(unix)]
use gwt_core::daemon::{ClientFrame, DaemonEndpoint, DaemonFrame};
use gwt_github::{client::ApiError, SpecOpsError};

use crate::cli::{CliEnv, CliParseError, DaemonCommand};

#[cfg(unix)]
const STATUS_PROBE_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) fn parse(args: &[String]) -> Result<DaemonCommand, CliParseError> {
    match args.first().map(String::as_str) {
        None | Some("start") => {
            ensure_no_extra_args(args.get(1..).unwrap_or(&[]))?;
            Ok(DaemonCommand::Start)
        }
        Some("status") => {
            ensure_no_extra_args(args.get(1..).unwrap_or(&[]))?;
            Ok(DaemonCommand::Status)
        }
        Some("subscribe") => {
            let channels: Vec<String> = args[1..].to_vec();
            if channels.is_empty() {
                return Err(CliParseError::Usage);
            }
            Ok(DaemonCommand::Subscribe { channels })
        }
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn ensure_no_extra_args(rest: &[String]) -> Result<(), CliParseError> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(CliParseError::Usage)
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: DaemonCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match cmd {
        DaemonCommand::Start => start_daemon(env, out),
        DaemonCommand::Status => report_status(env, out),
        DaemonCommand::Subscribe { channels } => subscribe_command(env, channels, out),
    }
}

fn config_error(message: impl Into<String>) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(message.into()))
}

fn resolve_scope(env: &impl CliEnv) -> Result<RuntimeScope, SpecOpsError> {
    let project_root = canonical_project_root(env.repo_path().to_path_buf());
    RuntimeScope::from_project_root(&project_root, RuntimeTarget::Host)
        .map_err(|err| config_error(format!("daemon scope resolution failed: {err}")))
}

fn canonical_project_root(path: PathBuf) -> PathBuf {
    dunce::canonicalize(&path).unwrap_or(path)
}

fn report_status<E: CliEnv>(env: &mut E, out: &mut String) -> Result<i32, SpecOpsError> {
    let scope = resolve_scope(env)?;
    let gwt_home = gwt_core::paths::gwt_home();
    let action = resolve_bootstrap_action(
        &gwt_home,
        &scope,
        DAEMON_PROTOCOL_VERSION,
        is_process_alive_pid,
    )
    .map_err(|err| config_error(err.to_string()))?;

    match action {
        DaemonBootstrapAction::Reuse(endpoint) => {
            let probe = probe_daemon_endpoint(&endpoint);
            out.push_str(&format!(
                "running pid={pid} bind={bind} version={version} probe={probe}\n",
                pid = endpoint.pid,
                bind = endpoint.bind,
                version = endpoint.daemon_version,
                probe = format_probe_result(&probe)
            ));
            Ok(0)
        }
        DaemonBootstrapAction::Spawn { endpoint_path } => {
            out.push_str(&format!(
                "stopped scope={repo_hash}/{worktree_hash} endpoint={path}\n",
                repo_hash = scope.repo_hash,
                worktree_hash = scope.worktree_hash,
                path = endpoint_path.display()
            ));
            Ok(0)
        }
    }
}

#[cfg(unix)]
fn probe_daemon_endpoint(endpoint: &DaemonEndpoint) -> Result<DaemonStatus, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| format!("tokio runtime build failed: {err}"))?;
    runtime.block_on(async {
        let connect = tokio::time::timeout(
            STATUS_PROBE_TIMEOUT,
            client::DaemonClient::connect(endpoint),
        )
        .await
        .map_err(|_| {
            format!(
                "probe timeout after {ms}ms",
                ms = STATUS_PROBE_TIMEOUT.as_millis()
            )
        })??;
        let mut client = connect;
        client
            .send_frame(&ClientFrame::Status)
            .await
            .map_err(|err| format!("status send failed: {err}"))?;
        let frame: DaemonFrame = tokio::time::timeout(STATUS_PROBE_TIMEOUT, client.read_frame())
            .await
            .map_err(|_| {
                format!(
                    "status read timeout after {ms}ms",
                    ms = STATUS_PROBE_TIMEOUT.as_millis()
                )
            })??;
        match frame {
            DaemonFrame::Status(status) => Ok(status),
            other => Err(format!("expected Status frame, got: {other:?}")),
        }
    })
}

#[cfg(not(unix))]
fn probe_daemon_endpoint(
    _endpoint: &gwt_core::daemon::DaemonEndpoint,
) -> Result<DaemonStatus, String> {
    Err("probe not implemented on this platform".to_string())
}

#[cfg(unix)]
fn subscribe_command<E: CliEnv>(
    env: &mut E,
    channels: Vec<String>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let scope = resolve_scope(env)?;
    let gwt_home = gwt_core::paths::gwt_home();
    let action = resolve_bootstrap_action(
        &gwt_home,
        &scope,
        DAEMON_PROTOCOL_VERSION,
        is_process_alive_pid,
    )
    .map_err(|err| config_error(err.to_string()))?;

    let endpoint = match action {
        DaemonBootstrapAction::Reuse(endpoint) => endpoint,
        DaemonBootstrapAction::Spawn { endpoint_path } => {
            out.push_str(&format!(
                "gwtd daemon subscribe: no daemon registered (endpoint={})\n",
                endpoint_path.display()
            ));
            return Ok(2);
        }
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| config_error(format!("tokio runtime build failed: {err}")))?;
    runtime.block_on(async {
        let mut client = client::DaemonClient::connect(&endpoint)
            .await
            .map_err(|err| config_error(format!("daemon connect failed: {err}")))?;
        client
            .send_frame(&ClientFrame::Subscribe { channels })
            .await
            .map_err(|err| config_error(format!("subscribe send failed: {err}")))?;

        // Drain frames until we observe the daemon's Subscribe ack.
        // Frames received before the Ack can be real `Event` payloads
        // because the per-channel forwarder is spawned before the
        // server enqueues the Ack — silently dropping the first frame
        // would cost the user the earliest event in the stream they
        // are watching.
        let writer = env.stdout();
        loop {
            let frame: DaemonFrame = client
                .read_frame()
                .await
                .map_err(|err| config_error(format!("subscribe ack failed: {err}")))?;
            match frame {
                DaemonFrame::Ack => break,
                DaemonFrame::Error { message } => {
                    return Err(config_error(format!(
                        "daemon rejected subscribe: {message}"
                    )));
                }
                other => {
                    let line = serde_json::to_string(&other)
                        .map_err(|err| config_error(format!("serialize event failed: {err}")))?;
                    writeln!(writer, "{line}")
                        .map_err(|err| config_error(format!("write stdout failed: {err}")))?;
                }
            }
        }

        loop {
            let frame: DaemonFrame = client
                .read_frame()
                .await
                .map_err(|err| config_error(format!("read event failed: {err}")))?;
            let line = serde_json::to_string(&frame)
                .map_err(|err| config_error(format!("serialize event failed: {err}")))?;
            writeln!(writer, "{line}")
                .map_err(|err| config_error(format!("write stdout failed: {err}")))?;
        }
    })
}

#[cfg(not(unix))]
fn subscribe_command<E: CliEnv>(
    _env: &mut E,
    _channels: Vec<String>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    out.push_str(
        "gwtd daemon subscribe: not implemented on this platform; \
         subscribe support requires Unix domain sockets.\n",
    );
    Ok(2)
}

#[cfg(unix)]
fn format_probe_result(result: &Result<DaemonStatus, String>) -> String {
    match result {
        Ok(status) => format!(
            "ok uptime={uptime}s channels={channels} connections={connections}",
            uptime = status.uptime_seconds,
            channels = status.broadcast_channels,
            connections = status.connections
        ),
        Err(err) => format!("failed:{err}"),
    }
}

#[cfg(not(unix))]
fn format_probe_result(result: &Result<DaemonStatus, String>) -> String {
    match result {
        Ok(_) => "ok".to_string(),
        Err(err) => format!("failed:{err}"),
    }
}

#[cfg(unix)]
fn start_daemon<E: CliEnv>(env: &mut E, out: &mut String) -> Result<i32, SpecOpsError> {
    let scope = resolve_scope(env)?;
    let gwt_home = gwt_core::paths::gwt_home();
    let action = resolve_bootstrap_action(
        &gwt_home,
        &scope,
        DAEMON_PROTOCOL_VERSION,
        is_process_alive_pid,
    )
    .map_err(|err| config_error(err.to_string()))?;

    match action {
        DaemonBootstrapAction::Reuse(endpoint) => {
            out.push_str(&format!(
                "daemon already running pid={pid} bind={bind}\n",
                pid = endpoint.pid,
                bind = endpoint.bind
            ));
            Ok(0)
        }
        DaemonBootstrapAction::Spawn { endpoint_path } => {
            server::serve_blocking(scope, endpoint_path, out)
        }
    }
}

#[cfg(not(unix))]
fn start_daemon<E: CliEnv>(_env: &mut E, out: &mut String) -> Result<i32, SpecOpsError> {
    out.push_str(
        "gwtd daemon start: long-running daemon mode is not yet implemented on this platform; \
         use `gwt hook ...` synchronous dispatch.\n",
    );
    Ok(2)
}

fn is_process_alive_pid(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(pid, 0) returns 0 if the process exists, -1 with
        // ESRCH if it does not. We never deliver a real signal.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        // EPERM means the process exists but we lack permission to signal it.
        matches!(err.raw_os_error(), Some(libc::EPERM))
    }
    #[cfg(not(unix))]
    {
        // Conservative fallback — assume alive so we never racily clobber
        // an in-flight daemon endpoint on an unsupported platform.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn parse_defaults_to_start_when_no_subcommand() {
        let cmd = parse(&[]).expect("parse");
        assert_eq!(cmd, DaemonCommand::Start);
    }

    #[test]
    fn parse_recognises_start_explicitly() {
        let cmd = parse(&[s("start")]).expect("parse");
        assert_eq!(cmd, DaemonCommand::Start);
    }

    #[test]
    fn parse_recognises_status() {
        let cmd = parse(&[s("status")]).expect("parse");
        assert_eq!(cmd, DaemonCommand::Status);
    }

    #[test]
    fn parse_recognises_subscribe_with_channels() {
        let cmd = parse(&[s("subscribe"), s("board"), s("runtime-status")]).expect("parse");
        assert_eq!(
            cmd,
            DaemonCommand::Subscribe {
                channels: vec!["board".to_string(), "runtime-status".to_string()]
            }
        );
    }

    #[test]
    fn parse_rejects_subscribe_without_channels() {
        let err = parse(&[s("subscribe")]).unwrap_err();
        assert!(matches!(err, CliParseError::Usage));
    }

    #[test]
    fn parse_rejects_unknown_subcommand() {
        let err = parse(&[s("foo")]).unwrap_err();
        assert!(matches!(err, CliParseError::UnknownSubcommand(_)));
    }

    #[test]
    fn parse_rejects_extra_args() {
        let err = parse(&[s("start"), s("--whatever")]).unwrap_err();
        assert!(matches!(err, CliParseError::Usage));
    }

    #[test]
    fn format_probe_result_err_includes_message() {
        let result: Result<DaemonStatus, String> = Err("connection refused".to_string());
        assert_eq!(format_probe_result(&result), "failed:connection refused");
    }

    #[cfg(unix)]
    #[test]
    fn probe_daemon_endpoint_fails_for_unreachable_bind() {
        use gwt_core::daemon::{DaemonEndpoint, RuntimeScope, RuntimeTarget};
        use tempfile::TempDir;

        let temp = TempDir::new().expect("tempdir");
        let scope = RuntimeScope::new(
            "abcdef0123456789",
            "feedfacecafebeef",
            temp.path().to_path_buf(),
            RuntimeTarget::Host,
        )
        .expect("scope");
        let bogus_socket = temp.path().join("does-not-exist.sock");
        let endpoint = DaemonEndpoint::new(
            scope,
            std::process::id(),
            bogus_socket.to_string_lossy().to_string(),
            "tok".to_string(),
            "test-daemon".to_string(),
        );
        let result = probe_daemon_endpoint(&endpoint);
        assert!(result.is_err(), "expected probe to fail for missing socket");
    }

    #[test]
    fn format_probe_result_ok_includes_uptime_and_channels() {
        let status = DaemonStatus {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            daemon_version: "9.14.0".to_string(),
            uptime_seconds: 12,
            broadcast_channels: 2,
            connections: 1,
        };
        let formatted = format_probe_result(&Ok(status));
        #[cfg(unix)]
        assert_eq!(formatted, "ok uptime=12s channels=2 connections=1");
        #[cfg(not(unix))]
        assert_eq!(formatted, "ok");
    }
}
