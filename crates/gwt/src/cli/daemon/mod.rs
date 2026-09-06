//! `gwtd daemon ...` family — long-running runtime daemon (SPEC-2077).
//!
//! - `mod.rs` (this file): argv parsing + dispatch + status reporting.
//! - `server.rs`: tokio-based IPC listener.
//! - `transport.rs`: the platform transport behind the listener (Unix
//!   domain socket on Unix, named pipe on Windows — Issue #3526).
//! - `subscribe_resolver.rs`: exact-first, read-only endpoint selection for
//!   bounded subscriptions from linked worktrees.
//!
//! The contract layer (`gwt_core::daemon::*`) defines the on-disk endpoint
//! file, handshake protocol, and `DaemonBootstrapAction`. `Start` honours
//! that contract: if a usable endpoint already exists for the cwd
//! [`RuntimeScope`], we exit 0 with a "already running" notice; otherwise
//! we generate a fresh `auth_token`, persist a new
//! [`gwt_core::daemon::DaemonEndpoint`], and
//! enter the listen loop.

pub(crate) mod broadcast;
pub mod client;
pub(crate) mod server;
mod subscribe_resolver;
pub(crate) mod transport;

use std::path::{Path, PathBuf};
use std::time::Duration;

use gwt_core::daemon::{
    resolve_bootstrap_action, ClientFrame, DaemonBootstrapAction, DaemonEndpoint, DaemonFrame,
    DaemonStatus, RuntimeScope, RuntimeTarget, DAEMON_PROTOCOL_VERSION,
};
use gwt_github::{client::ApiError, SpecOpsError};

use crate::cli::{CliEnv, CliParseError, DaemonCommand};

const STATUS_PROBE_TIMEOUT: Duration = Duration::from_secs(1);

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
                project_root: None,
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
            project_root,
            timeout_seconds,
        } => subscribe_command(env, project_root.as_deref(), channels, timeout_seconds, out),
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

/// Resolve the authority requested by `daemon.subscribe`.
///
/// An explicit root is an authorization boundary: it must exist and be a
/// directory, and it must never degrade to the caller's cwd. Omitting the
/// root retains the pre-#3596 cwd-derived scope unchanged.
fn resolve_subscribe_scope(
    env: &impl CliEnv,
    project_root: Option<&Path>,
) -> Result<RuntimeScope, SpecOpsError> {
    let Some(requested) = project_root else {
        return resolve_scope(env);
    };
    let canonical = dunce::canonicalize(requested).map_err(|error| {
        config_error(format!(
            "daemon subscribe project_root {} is unavailable: {error}",
            requested.display()
        ))
    })?;
    if !canonical.is_dir() {
        return Err(config_error(format!(
            "daemon subscribe project_root {} is not a directory",
            requested.display()
        )));
    }
    let project_root = gwt_core::paths::resolve_current_worktree_root(&canonical);
    RuntimeScope::from_project_root(&project_root, RuntimeTarget::Host)
        .map_err(|err| config_error(format!("daemon subscribe scope resolution failed: {err}")))
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
            let state = match unregistered_daemon_evidence(&endpoint_path) {
                Some(evidence) => format!("unregistered {evidence}"),
                None => "stopped".to_string(),
            };
            out.push_str(&format!(
                "{state} scope={repo_hash}/{worktree_hash} endpoint={path}\n",
                repo_hash = scope.repo_hash,
                worktree_hash = scope.worktree_hash,
                path = endpoint_path.display()
            ));
            Ok(0)
        }
    }
}

/// Issue #2338 AC-B: evidence that a daemon is serving this scope even though
/// no usable descriptor names it, or `None` when nothing is running.
///
/// Without this, `stopped` covers two states a reader must act on differently:
/// no daemon at all (start one) versus a daemon that is up but undiscoverable
/// (find out what erased or skewed its descriptor). The production incident
/// was the second one, reported as the first for hours.
fn unregistered_daemon_evidence(endpoint_path: &std::path::Path) -> Option<String> {
    // A descriptor this gwtd cannot use — wrong protocol, wrong scope — but
    // whose owner is still running. Since Issue #2338 AC-A such a descriptor
    // is preserved rather than deleted, so it is here to be read.
    if let Some(endpoint) = std::fs::read(endpoint_path)
        .ok()
        .and_then(|payload| serde_json::from_slice::<DaemonEndpoint>(&payload).ok())
    {
        if endpoint.has_live_owner(is_process_alive_pid) {
            return Some(format!(
                "pid={pid} bind={bind} reason=endpoint-unusable-for-this-client",
                pid = endpoint.pid,
                bind = endpoint.bind
            ));
        }
        return None;
    }

    // No readable descriptor, but the scope's canonical socket still answers:
    // exactly the shape the production machine was in.
    let socket = gwt_core::daemon::resolve_daemon_socket_path(endpoint_path).ok()?;
    transport::bind_is_served(&socket.path.to_string_lossy()).then(|| {
        format!(
            "socket={path} reason=endpoint-missing",
            path = socket.path.display()
        )
    })
}

/// Issue #2338 AC-D: drop the descriptors earlier gwt generations left behind
/// in this project's daemon directory.
///
/// Runs at daemon start because that is the one moment a writer legitimately
/// owns this directory, and because the sweep only ever removes descriptors
/// whose owner is provably gone — it cannot strand the daemon about to bind.
/// Failures are logged, never fatal: a daemon that can serve must not refuse
/// to start over housekeeping.
fn sweep_past_generation_endpoints(daemon_dir: &std::path::Path) {
    match gwt_core::daemon::sweep_dead_endpoints(
        daemon_dir,
        is_process_alive_pid,
        transport::bind_is_served,
    ) {
        Ok(removed) if !removed.is_empty() => {
            tracing::info!(
                removed = removed.len(),
                dir = %daemon_dir.display(),
                "gwtd daemon: swept past-generation endpoint descriptors (#2338 AC-D)"
            );
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(
                dir = %daemon_dir.display(),
                "gwtd daemon: endpoint descriptor sweep failed: {error}"
            );
        }
    }
}

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

fn subscribe_command<E: CliEnv>(
    env: &mut E,
    project_root: Option<&Path>,
    channels: Vec<String>,
    timeout_seconds: Option<u64>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let scope = resolve_subscribe_scope(env, project_root)?;
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

fn start_daemon<E: CliEnv>(env: &mut E, out: &mut String) -> Result<i32, SpecOpsError> {
    let scope = resolve_scope(env)?;
    let gwt_home = gwt_core::paths::gwt_home();
    sweep_past_generation_endpoints(&scope.daemon_dir(&gwt_home));
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

// Liveness probe lives in `crate::process::is_process_alive` so the
// three daemon-related callers (this file, daemon_publisher, main)
// share one definition. Issue #2338 removed the last divergent copy
// along with the GUI front door's endpoint-slot handling.
use crate::process::is_process_alive as is_process_alive_pid;

#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    #[cfg(unix)]
    fn spawn_fake_subscribe_daemon(
        listener: std::os::unix::net::UnixListener,
        endpoint: DaemonEndpoint,
        expected_probe_connections: usize,
    ) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<usize>) {
        use std::{
            io::{BufRead, BufReader, ErrorKind, Write},
            sync::mpsc,
            thread,
            time::Instant,
        };

        listener
            .set_nonblocking(true)
            .expect("set fake daemon nonblocking");
        let (release_server, await_command_exit) = mpsc::channel();
        let server = thread::spawn(move || {
            let mut connection_count = 0;
            let expected_connection_count = expected_probe_connections + 1;
            for connection_index in 0..expected_connection_count {
                let deadline = Instant::now() + Duration::from_secs(2);
                let (mut stream, _) = loop {
                    match await_command_exit.try_recv() {
                        Ok(()) | Err(mpsc::TryRecvError::Disconnected) => {
                            return connection_count;
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(err) if err.kind() == ErrorKind::WouldBlock => {
                            if Instant::now() >= deadline {
                                return connection_count;
                            }
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(err) => panic!("accept subscriber: {err}"),
                    }
                };
                stream
                    .set_nonblocking(false)
                    .expect("restore subscriber blocking mode");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("set subscriber read timeout");
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("set subscriber write timeout");
                connection_count += 1;

                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut line = String::new();
                reader.read_line(&mut line).expect("read handshake");
                let handshake: gwt_core::daemon::IpcHandshakeRequest =
                    serde_json::from_str(line.trim_end()).expect("parse handshake");
                assert_eq!(handshake.scope, endpoint.scope);
                assert_eq!(handshake.auth_token, endpoint.auth_token);
                let response = gwt_core::daemon::IpcHandshakeResponse {
                    protocol_version: endpoint.protocol_version,
                    daemon_version: endpoint.daemon_version.clone(),
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

                if connection_index < expected_probe_connections {
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
            connection_count
        });
        (release_server, server)
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
                project_root: None,
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
                project_root: None,
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

    /// Issue #3526 AC-1 / AC-2: the daemon transport layer must compile on
    /// every supported host. A crate-level Unix-only module gate or a
    /// platform stub means Windows silently loses `daemon.start`,
    /// `daemon.subscribe`, the status probe, and every daemon-backed
    /// control publish. The needles are assembled at runtime so this test
    /// does not match its own source text.
    #[test]
    fn daemon_modules_have_no_platform_gate_or_platform_stub() {
        let sources = [
            ("cli/daemon/mod.rs", include_str!("mod.rs")),
            ("cli/daemon/server.rs", include_str!("server.rs")),
            ("cli/daemon/client.rs", include_str!("client.rs")),
            ("cli/daemon/broadcast.rs", include_str!("broadcast.rs")),
            (
                "cli/daemon/subscribe_resolver.rs",
                include_str!("subscribe_resolver.rs"),
            ),
            (
                "daemon_publisher.rs",
                include_str!("../../daemon_publisher.rs"),
            ),
            (
                "daemon_subscriber.rs",
                include_str!("../../daemon_subscriber.rs"),
            ),
            (
                "daemon_supervisor.rs",
                include_str!("../../daemon_supervisor.rs"),
            ),
            ("cli/issue.rs", include_str!("../issue.rs")),
            ("cli/workspace.rs", include_str!("../workspace.rs")),
            ("lib.rs", include_str!("../../lib.rs")),
        ];
        let module_gate = format!("#![cfg({})]", "unix");
        let stubs = ["not implemented", "not yet implemented", "unavailable"]
            .map(|prefix| format!("{prefix} on this {}", "platform"));
        for (name, source) in sources {
            assert!(
                !source.contains(&module_gate),
                "{name}: daemon module must not be gated to Unix"
            );
            for stub in &stubs {
                assert!(
                    !source.contains(stub.as_str()),
                    "{name}: daemon surface must not carry the platform stub {stub:?}"
                );
            }
        }
    }

    #[test]
    fn format_probe_result_err_includes_message() {
        let result: Result<DaemonStatus, String> = Err("connection refused".to_string());
        assert_eq!(format_probe_result(&result), "failed:connection refused");
    }

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

    /// Issue #2338 AC-B: on the production machine the daemon process was
    /// running and `daemon.status` still said `stopped`, because the status
    /// path only ever asks "is there a usable descriptor". A reader cannot
    /// tell "nobody is serving this project" from "somebody is serving it but
    /// the descriptor is gone or unusable" — and only the second one is a bug
    /// worth chasing.
    #[cfg(unix)]
    #[test]
    fn daemon_status_separates_an_unregistered_live_daemon_from_a_stopped_one() {
        use std::os::unix::net::UnixListener;

        use gwt_core::{
            daemon::{persist_endpoint, resolve_daemon_socket_path},
            test_support::ScopedGwtHome,
        };

        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::TempDir::new().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(temp.path().join("gwt-home"));
        let project_root = temp.path().join("project");
        std::fs::create_dir_all(&project_root).expect("project root");
        let mut env = crate::cli::TestEnv::new(project_root.clone());
        env.repo_path = project_root.clone();
        let scope = resolve_scope(&env).expect("scope");
        let endpoint_path = scope.endpoint_path(&gwt_core::paths::gwt_home());

        let mut stopped = String::new();
        run(&mut env, DaemonCommand::Status, &mut stopped).expect("status with nothing running");
        assert!(
            stopped.starts_with("stopped "),
            "no daemon and no socket must still read as stopped: {stopped}"
        );

        // A live owner whose descriptor this gwtd cannot use (protocol skew).
        let mut skewed = DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            temp.path().join("skewed.sock").display().to_string(),
            "skewed-token".to_string(),
            "test-daemon".to_string(),
        );
        skewed.protocol_version = DAEMON_PROTOCOL_VERSION + 1;
        persist_endpoint(&endpoint_path, &skewed).expect("persist skewed endpoint");
        let mut skewed_out = String::new();
        run(&mut env, DaemonCommand::Status, &mut skewed_out).expect("status with skewed endpoint");
        assert!(
            skewed_out.starts_with("unregistered "),
            "a live owner with an unusable descriptor is not `stopped`: {skewed_out}"
        );
        assert!(
            skewed_out.contains(&format!("pid={}", std::process::id())),
            "the owner pid is the whole point of the report: {skewed_out}"
        );

        // The production shape: descriptor gone, daemon still serving.
        std::fs::remove_file(&endpoint_path).expect("remove endpoint");
        let socket = resolve_daemon_socket_path(&endpoint_path).expect("socket path");
        if let Some(parent) = socket.path.parent() {
            std::fs::create_dir_all(parent).expect("socket parent");
        }
        let _listener = UnixListener::bind(&socket.path).expect("bind canonical socket");
        let mut serving_out = String::new();
        run(&mut env, DaemonCommand::Status, &mut serving_out)
            .expect("status with a served socket and no descriptor");
        assert!(
            serving_out.starts_with("unregistered "),
            "a served socket with no descriptor is not `stopped`: {serving_out}"
        );
        assert!(
            serving_out.contains(&socket.path.display().to_string()),
            "the report must name the socket that proves it: {serving_out}"
        );
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
        assert_eq!(formatted, "ok uptime=12s channels=2 connections=1");
    }

    #[cfg(unix)]
    #[test]
    fn subscribe_command_reaches_unique_same_repo_sibling_daemon() {
        use std::os::unix::net::UnixListener;

        use gwt_core::{
            daemon::{persist_endpoint, RuntimeTarget},
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

        let (release_server, server) = spawn_fake_subscribe_daemon(listener, endpoint, 1);

        let mut output = String::new();
        let exit = run(
            &mut env,
            DaemonCommand::Subscribe {
                channels: vec!["issue-monitor".to_string()],
                project_root: None,
                timeout_seconds: Some(1),
            },
            &mut output,
        )
        .expect("subscribe command");

        assert_eq!(exit, 0);
        assert!(output.is_empty());
        release_server.send(()).expect("release server");
        assert_eq!(server.join().expect("server thread"), 2);
    }

    /// Issue #3596: JSON dispatch may originate outside the requested
    /// project. The explicit root must select that project's exact endpoint,
    /// never the process cwd's daemon scope.
    #[cfg(unix)]
    #[test]
    fn subscribe_run_uses_explicit_project_root_instead_of_env_cwd() {
        use std::os::unix::net::UnixListener;

        use gwt_core::{
            daemon::{persist_endpoint, RuntimeTarget},
            test_support::ScopedGwtHome,
        };

        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::TempDir::new().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(temp.path().join("gwt-home"));
        let cwd_root = temp.path().join("cwd-project");
        let explicit_root = temp.path().join("explicit-project");
        std::fs::create_dir_all(&cwd_root).expect("cwd root");
        std::fs::create_dir_all(&explicit_root).expect("explicit root");
        let mut env = crate::cli::TestEnv::new(cwd_root.clone());
        env.repo_path = cwd_root.clone();

        let cwd_scope =
            RuntimeScope::from_project_root(&cwd_root, RuntimeTarget::Host).expect("cwd scope");
        let explicit_scope = RuntimeScope::from_project_root(&explicit_root, RuntimeTarget::Host)
            .expect("explicit scope");
        let cwd_socket_path = temp.path().join("cwd.sock");
        let explicit_socket_path = temp.path().join("explicit.sock");
        let cwd_listener = UnixListener::bind(&cwd_socket_path).expect("bind cwd socket");
        let explicit_listener =
            UnixListener::bind(&explicit_socket_path).expect("bind explicit socket");
        let cwd_endpoint = DaemonEndpoint::new(
            cwd_scope.clone(),
            std::process::id(),
            cwd_socket_path.display().to_string(),
            "cwd-secret".to_string(),
            "test-daemon".to_string(),
        );
        let explicit_endpoint = DaemonEndpoint::new(
            explicit_scope.clone(),
            std::process::id(),
            explicit_socket_path.display().to_string(),
            "explicit-secret".to_string(),
            "test-daemon".to_string(),
        );
        persist_endpoint(
            &cwd_scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &cwd_endpoint,
        )
        .expect("persist cwd endpoint");
        persist_endpoint(
            &explicit_scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &explicit_endpoint,
        )
        .expect("persist explicit endpoint");
        let (release_cwd_server, cwd_server) =
            spawn_fake_subscribe_daemon(cwd_listener, cwd_endpoint, 0);
        let (release_explicit_server, explicit_server) =
            spawn_fake_subscribe_daemon(explicit_listener, explicit_endpoint, 0);

        let mut output = String::new();
        let result = run(
            &mut env,
            DaemonCommand::Subscribe {
                channels: vec!["issue-monitor".to_string()],
                project_root: Some(explicit_root),
                timeout_seconds: Some(1),
            },
            &mut output,
        );

        let _ = release_cwd_server.send(());
        let _ = release_explicit_server.send(());
        let cwd_connection_count = cwd_server.join().expect("cwd server thread");
        let explicit_connection_count = explicit_server.join().expect("explicit server thread");
        assert_eq!(result.expect("explicit subscribe command"), 0);
        assert!(output.is_empty(), "unexpected output: {output}");
        assert_eq!(
            (explicit_connection_count, cwd_connection_count),
            (1, 0),
            "explicit project_root must select only the explicit endpoint"
        );
    }

    #[cfg(unix)]
    fn assert_invalid_explicit_project_root_is_rejected(case: &str) {
        use std::os::unix::net::UnixListener;

        use gwt_core::{
            daemon::{persist_endpoint, RuntimeTarget},
            test_support::ScopedGwtHome,
        };

        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::TempDir::new().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(temp.path().join("gwt-home"));
        let cwd_root = temp.path().join("cwd-project");
        std::fs::create_dir_all(&cwd_root).expect("cwd root");
        let mut env = crate::cli::TestEnv::new(cwd_root.clone());
        env.repo_path = cwd_root.clone();
        let cwd_scope =
            RuntimeScope::from_project_root(&cwd_root, RuntimeTarget::Host).expect("cwd scope");
        let project_root = temp.path().join(format!("invalid-{case}"));
        if case == "file" {
            std::fs::write(&project_root, "not a project").expect("write file root");
        }

        let cwd_socket_path = temp.path().join(format!("cwd-{case}.sock"));
        let cwd_listener = UnixListener::bind(&cwd_socket_path).expect("bind cwd socket");
        let cwd_endpoint = DaemonEndpoint::new(
            cwd_scope.clone(),
            std::process::id(),
            cwd_socket_path.display().to_string(),
            format!("cwd-{case}-secret"),
            "test-daemon".to_string(),
        );
        persist_endpoint(
            &cwd_scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &cwd_endpoint,
        )
        .expect("persist cwd endpoint");
        let (release_cwd_server, cwd_server) =
            spawn_fake_subscribe_daemon(cwd_listener, cwd_endpoint, 0);
        let mut output = String::new();
        let result = run(
            &mut env,
            DaemonCommand::Subscribe {
                channels: vec!["issue-monitor".to_string()],
                project_root: Some(project_root.clone()),
                timeout_seconds: Some(1),
            },
            &mut output,
        );

        let _ = release_cwd_server.send(());
        let cwd_connection_count = cwd_server.join().expect("cwd server thread");
        assert!(
            result.is_err() && cwd_connection_count == 0,
            "invalid explicit root {} must fail before contacting the cwd endpoint; result={result:?}, cwd_connections={cwd_connection_count}, output={output}",
            project_root.display(),
        );
    }

    /// Issue #3596: a nonexistent explicit root is a caller error and must
    /// not silently fall back to a valid daemon associated with process cwd.
    #[cfg(unix)]
    #[test]
    fn subscribe_run_rejects_nonexistent_project_root_without_contacting_cwd() {
        assert_invalid_explicit_project_root_is_rejected("missing");
    }

    /// Issue #3596: a file cannot identify a project root and must not
    /// silently fall back to a valid daemon associated with process cwd.
    #[cfg(unix)]
    #[test]
    fn subscribe_run_rejects_file_project_root_without_contacting_cwd() {
        assert_invalid_explicit_project_root_is_rejected("file");
    }
}
