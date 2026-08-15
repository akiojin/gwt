//! `gwtd daemon ...` family — long-running runtime daemon (SPEC-2077).
//!
//! - `mod.rs` (this file): argv parsing + dispatch + status reporting.
//! - `server.rs`: tokio-based IPC listener (Unix domain socket today;
//!   Windows named-pipe support is a follow-up).
//! - `subscribe_resolver.rs`: exact-first, read-only endpoint selection for
//!   bounded Unix subscriptions from linked worktrees.
//!
//! The contract layer (`gwt_core::daemon::*`) defines the on-disk endpoint
//! file, handshake protocol, and `DaemonBootstrapAction`. `Start` honours
//! that contract: if a usable endpoint already exists for the cwd
//! [`RuntimeScope`], we exit 0 with a "already running" notice; otherwise
//! we generate a fresh `auth_token`, persist a new
//! [`gwt_core::daemon::DaemonEndpoint`], and
//! enter the listen loop.

#[cfg(unix)]
pub(crate) mod broadcast;
#[cfg(unix)]
pub mod client;
#[cfg(unix)]
pub(crate) mod server;
#[cfg(unix)]
mod subscribe_resolver;

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

#[cfg(unix)]
fn subscribe_deadline_from(
    now: tokio::time::Instant,
    timeout_seconds: u64,
) -> Option<tokio::time::Instant> {
    now.checked_add(Duration::from_secs(timeout_seconds))
}

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
            let mut channels = Vec::new();
            let mut timeout_seconds = None;
            let mut rest = args[1..].iter();
            while let Some(arg) = rest.next() {
                if arg == "--timeout-seconds" {
                    let value = rest.next().ok_or(CliParseError::Usage)?;
                    let seconds: u64 = value
                        .parse()
                        .map_err(|_| CliParseError::InvalidNumber(value.clone()))?;
                    if seconds == 0 {
                        return Err(CliParseError::InvalidValue {
                            flag: "--timeout-seconds",
                            reason: "must be at least 1 second",
                        });
                    }
                    timeout_seconds = Some(seconds);
                } else {
                    channels.push(arg.clone());
                }
            }
            if channels.is_empty() {
                return Err(CliParseError::Usage);
            }
            Ok(DaemonCommand::Subscribe {
                channels,
                timeout_seconds,
            })
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
        DaemonCommand::Subscribe {
            channels,
            timeout_seconds,
        } => subscribe_command(env, channels, timeout_seconds, out),
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
    timeout_seconds: Option<u64>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let scope = resolve_scope(env)?;
    let gwt_home = gwt_core::paths::gwt_home();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| config_error(format!("tokio runtime build failed: {err}")))?;
    let resolved = match runtime.block_on(subscribe_resolver::resolve(
        &gwt_home,
        &scope,
        DAEMON_PROTOCOL_VERSION,
        is_process_alive_pid,
    )) {
        Ok(resolved) => resolved,
        Err(failure) => {
            out.push_str(&failure.to_string());
            out.push('\n');
            return Ok(2);
        }
    };
    let endpoint = resolved;

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

        // FR-025: a bounded run ends on a wall-clock deadline, not a per-read
        // timeout. Busy channels deliver frames continuously, so a per-read
        // timeout would never fire and the caller would never get its turn
        // back.
        let deadline = timeout_seconds
            .and_then(|seconds| subscribe_deadline_from(tokio::time::Instant::now(), seconds));
        loop {
            let frame: DaemonFrame = match deadline {
                Some(deadline) => {
                    match tokio::time::timeout_at(deadline, client.read_frame()).await {
                        Ok(frame) => frame
                            .map_err(|err| config_error(format!("read event failed: {err}")))?,
                        Err(_) => return Ok(0),
                    }
                }
                None => client
                    .read_frame()
                    .await
                    .map_err(|err| config_error(format!("read event failed: {err}")))?,
            };
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
    _timeout_seconds: Option<u64>,
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
            // Pass `env.stdout()` directly so `serve_blocking` can
            // flush its readiness lines while the serve loop is still
            // running. Returning the buffer-only `out` here would mean
            // those lines are visible only after shutdown.
            server::serve_blocking(scope, endpoint_path, env.stdout())
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

// Liveness probe lives in `crate::process::is_process_alive` so the
// three daemon-related callers (this file, daemon_publisher, main)
// share one definition. The narrow `|pid| pid == self.pid` predicate
// used by `prepare_daemon_front_door_for_path` is intentionally NOT
// the same function; see Issue #2338.
use crate::process::is_process_alive as is_process_alive_pid;

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
                channels: vec!["board".to_string(), "runtime-status".to_string()],
                timeout_seconds: None,
            }
        );
    }

    #[test]
    fn parse_rejects_subscribe_without_channels() {
        let err = parse(&[s("subscribe")]).unwrap_err();
        assert!(matches!(err, CliParseError::Usage));
    }

    /// SPEC-3431 FR-025: an unattended agent runs subscribe → reconcile in a
    /// loop, so it needs the read to end on its own. Without this the read
    /// loop never returns and "run a bounded subscribe" is an instruction the
    /// runtime cannot honor.
    #[test]
    fn parse_recognises_subscribe_timeout() {
        let cmd =
            parse(&[s("subscribe"), s("board"), s("--timeout-seconds"), s("30")]).expect("parse");
        assert_eq!(
            cmd,
            DaemonCommand::Subscribe {
                channels: vec!["board".to_string()],
                timeout_seconds: Some(30),
            }
        );
    }

    #[test]
    fn parse_rejects_a_zero_or_unparsable_subscribe_timeout() {
        for value in ["0", "-1", "soon"] {
            assert!(
                parse(&[s("subscribe"), s("board"), s("--timeout-seconds"), s(value)]).is_err(),
                "timeout {value} must be rejected"
            );
        }
        assert!(parse(&[s("subscribe"), s("board"), s("--timeout-seconds")]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn subscribe_deadline_treats_instant_overflow_as_effectively_unbounded() {
        let now = tokio::time::Instant::now();

        assert_eq!(
            subscribe_deadline_from(now, 30),
            now.checked_add(Duration::from_secs(30))
        );
        assert_eq!(subscribe_deadline_from(now, u64::MAX), None);
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
            issue_monitor: None,
        };
        let formatted = format_probe_result(&Ok(status));
        #[cfg(unix)]
        assert_eq!(formatted, "ok uptime=12s channels=2 connections=1");
        #[cfg(not(unix))]
        assert_eq!(formatted, "ok");
    }

    #[cfg(unix)]
    #[test]
    fn subscribe_command_reaches_unique_same_repo_sibling_daemon() {
        use std::{
            io::{BufRead, BufReader, Write},
            os::unix::net::UnixListener,
            sync::mpsc,
            thread,
        };

        use gwt_core::{
            daemon::{persist_endpoint, IpcHandshakeRequest, IpcHandshakeResponse, RuntimeTarget},
            test_support::ScopedGwtHome,
        };

        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::TempDir::new().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(temp.path());
        let caller_root = temp.path().join("caller");
        let sibling_root = temp.path().join("sibling");
        std::fs::create_dir_all(&caller_root).expect("caller root");
        std::fs::create_dir_all(&sibling_root).expect("sibling root");
        let mut env = crate::cli::TestEnv::new(caller_root.clone());
        env.repo_path = caller_root;
        let caller_scope = resolve_scope(&env).expect("caller scope");
        let sibling_scope = RuntimeScope::new(
            caller_scope.repo_hash.clone(),
            "sibling-worktree",
            sibling_root,
            RuntimeTarget::Host,
        )
        .expect("sibling scope");
        let socket_path = temp.path().join("sibling.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind sibling socket");
        let endpoint = DaemonEndpoint::new(
            sibling_scope.clone(),
            std::process::id(),
            socket_path.display().to_string(),
            "sibling-secret".to_string(),
            "test-daemon".to_string(),
        );
        persist_endpoint(
            &sibling_scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &endpoint,
        )
        .expect("persist sibling endpoint");

        let (release_server, await_command_exit) = mpsc::channel();
        let server_endpoint = endpoint.clone();
        let server = thread::spawn(move || {
            for connection_index in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept subscriber");
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut line = String::new();
                reader.read_line(&mut line).expect("read handshake");
                let handshake: IpcHandshakeRequest =
                    serde_json::from_str(line.trim_end()).expect("parse handshake");
                assert_eq!(handshake.scope, server_endpoint.scope);
                assert_eq!(handshake.auth_token, server_endpoint.auth_token);
                let response = IpcHandshakeResponse {
                    protocol_version: server_endpoint.protocol_version,
                    daemon_version: server_endpoint.daemon_version.clone(),
                    accepted: true,
                    rejection_reason: None,
                };
                writeln!(
                    stream,
                    "{}",
                    serde_json::to_string(&response).expect("serialize response")
                )
                .expect("write handshake response");
                stream.flush().expect("flush handshake response");

                if connection_index == 0 {
                    continue;
                }
                line.clear();
                reader.read_line(&mut line).expect("read subscribe");
                assert!(matches!(
                    serde_json::from_str::<ClientFrame>(line.trim_end()).expect("parse subscribe"),
                    ClientFrame::Subscribe { .. }
                ));
                writeln!(
                    stream,
                    "{}",
                    serde_json::to_string(&DaemonFrame::Ack).expect("serialize ack")
                )
                .expect("write ack");
                stream.flush().expect("flush ack");
                await_command_exit
                    .recv_timeout(Duration::from_secs(5))
                    .expect("command must finish before server closes subscriber");
            }
        });

        let mut output = String::new();
        let exit = subscribe_command(
            &mut env,
            vec!["issue-monitor".to_string()],
            Some(1),
            &mut output,
        )
        .expect("subscribe command");

        assert_eq!(exit, 0);
        assert!(output.is_empty());
        release_server.send(()).expect("release server");
        server.join().expect("server thread");
    }
}
