#![cfg(unix)]
//! Real-`gwtd` regression coverage for Issue #3596.
//!
//! `daemon.subscribe` used to discard `params.project_root` and derive its
//! runtime authority from the caller's cwd. These tests publish conflicting
//! live endpoint records for a caller and an explicit target, then exercise
//! the complete stdin-JSON -> endpoint resolution -> Unix socket path.

use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use gwt_core::{
    daemon::{
        persist_endpoint, ClientFrame, DaemonEndpoint, DaemonFrame, IpcHandshakeRequest,
        IpcHandshakeResponse, RuntimeScope, RuntimeTarget,
    },
    process::hidden_command,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tempfile::TempDir;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(8);
const SERVER_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(3);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct Fixture {
    _root: TempDir,
    home: PathBuf,
    gwt_home: PathBuf,
    caller: PathBuf,
    target: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = TempDir::new().expect("fixture tempdir");
        let home = root.path().join("home");
        let gwt_home = home.join(".gwt");
        let caller = root.path().join("caller");
        let target = root.path().join("target");
        for dir in [&home, &gwt_home, &caller, &target] {
            fs::create_dir_all(dir).unwrap_or_else(|error| {
                panic!("create fixture directory {}: {error}", dir.display())
            });
        }
        Self {
            _root: root,
            home,
            gwt_home,
            caller: canonical(&caller),
            target: canonical(&target),
        }
    }

    fn gwtd(&self) -> Command {
        let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
        // The surrounding managed session must not redirect the child to the
        // developer checkout's real runtime or coordination state.
        for key in [
            "GWT_BIN_PATH",
            "GWT_BROWSER_URL_FILE",
            "GWT_DAEMON_SOCKET_DIR",
            "GWT_HOOK_BIN",
            "GWT_HOOK_FORWARD_TOKEN",
            "GWT_HOOK_FORWARD_URL",
            "GWT_PROJECT_ROOT",
            "GWT_REPO_HASH",
            "GWT_SESSION_ID",
            "GWT_SESSION_KIND",
            "GWT_SESSION_RUNTIME_PATH",
            "GWT_WORKTREE_HASH",
        ] {
            command.env_remove(key);
        }
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .current_dir(&self.caller);
        command
    }
}

struct CommandOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

/// Run one real `gwtd` JSON operation without allowing a broken subscribe to
/// wedge the test process forever.
fn run_envelope(command: &mut Command, envelope: &Value) -> CommandOutput {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gwtd");
    {
        let mut stdin = child.stdin.take().expect("gwtd stdin");
        serde_json::to_writer(&mut stdin, envelope).expect("serialize JSON envelope");
        stdin.write_all(b"\n").expect("terminate JSON envelope");
    }

    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("gwtd did not finish within {COMMAND_TIMEOUT:?}: {envelope}");
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(error) => panic!("wait for gwtd: {error}"),
        }
    }
    let output = child.wait_with_output().expect("collect gwtd output");
    CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

struct FakeDaemon {
    endpoint: DaemonEndpoint,
    handshakes: Arc<AtomicUsize>,
    subscriptions: Arc<AtomicUsize>,
    errors: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeDaemon {
    fn start(gwt_home: &Path, project_root: &Path, label: &str, event_payload: Value) -> Self {
        let scope = RuntimeScope::from_project_root(project_root, RuntimeTarget::Host)
            .unwrap_or_else(|error| panic!("resolve {label} scope: {error}"));
        let socket_path = project_root.join(format!("{label}.sock"));
        let _ = fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path)
            .unwrap_or_else(|error| panic!("bind {label} socket: {error}"));
        listener
            .set_nonblocking(true)
            .expect("set fake daemon listener nonblocking");

        let endpoint = DaemonEndpoint::new(
            scope,
            std::process::id(),
            socket_path.display().to_string(),
            format!("{label}-token"),
            format!("{label}-daemon"),
        );
        persist_endpoint(&endpoint.scope.endpoint_path(gwt_home), &endpoint)
            .unwrap_or_else(|error| panic!("persist {label} endpoint: {error}"));

        let handshakes = Arc::new(AtomicUsize::new(0));
        let subscriptions = Arc::new(AtomicUsize::new(0));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let server_endpoint = endpoint.clone();
        let server_handshakes = Arc::clone(&handshakes);
        let server_subscriptions = Arc::clone(&subscriptions);
        let server_errors = Arc::clone(&errors);
        let server_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name(format!("fake-daemon-{label}"))
            .spawn(move || {
                let deadline = Instant::now() + SERVER_TIMEOUT;
                while !server_stop.load(Ordering::Acquire) && Instant::now() < deadline {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            // `finish` wakes accept with an empty connection.
                            // Re-check the stop flag after accept so that wake
                            // socket can never enter the protocol handler.
                            if server_stop.load(Ordering::Acquire) {
                                break;
                            }
                            if let Err(error) = handle_connection(
                                stream,
                                &server_endpoint,
                                &event_payload,
                                &server_handshakes,
                                &server_subscriptions,
                            ) {
                                server_errors
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .push(error);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(POLL_INTERVAL);
                        }
                        Err(error) => {
                            server_errors
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push(format!("accept failed: {error}"));
                            break;
                        }
                    }
                }
            })
            .expect("spawn fake daemon");

        Self {
            endpoint,
            handshakes,
            subscriptions,
            errors,
            stop,
            thread: Some(thread),
        }
    }

    fn handshake_count(&self) -> usize {
        self.handshakes.load(Ordering::Acquire)
    }

    fn subscription_count(&self) -> usize {
        self.subscriptions.load(Ordering::Acquire)
    }

    /// Stop the fake server and make every background failure part of the
    /// test result. Consuming `self` prevents a successful test from silently
    /// falling back to Drop's best-effort cleanup path.
    fn finish(mut self) {
        self.shutdown_and_assert();
    }

    fn shutdown_and_assert(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = UnixStream::connect(&self.endpoint.bind);

        let join_result = self
            .thread
            .take()
            .expect("fake daemon finish called exactly once")
            .join();
        if let Err(panic) = join_result {
            self.remove_socket_best_effort();
            std::panic::resume_unwind(panic);
        }

        let errors = self
            .errors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if !errors.is_empty() {
            self.remove_socket_best_effort();
            panic!("fake daemon errors: {errors:?}");
        }

        match fs::remove_file(&self.endpoint.bind) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("remove fake daemon socket {}: {error}", self.endpoint.bind),
        }
    }

    fn remove_socket_best_effort(&self) {
        let _ = fs::remove_file(&self.endpoint.bind);
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        if self.thread.is_none() {
            return;
        }
        // Panic-path fallback only. The normal path must call `finish`, which
        // observes join failures and protocol errors instead of discarding
        // them.
        self.stop.store(true, Ordering::Release);
        let _ = UnixStream::connect(&self.endpoint.bind);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.remove_socket_best_effort();
    }
}

fn handle_connection(
    mut stream: UnixStream,
    endpoint: &DaemonEndpoint,
    event_payload: &Value,
    handshakes: &AtomicUsize,
    subscriptions: &AtomicUsize,
) -> Result<(), String> {
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("set stream blocking: {error}"))?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("set write timeout: {error}"))?;
    let read_stream = stream
        .try_clone()
        .map_err(|error| format!("clone stream: {error}"))?;
    let mut reader = BufReader::new(read_stream);

    let Some(request) = read_json_line::<IpcHandshakeRequest>(&mut reader)? else {
        return Ok(());
    };
    handshakes.fetch_add(1, Ordering::AcqRel);
    if request.protocol_version != endpoint.protocol_version
        || request.auth_token != endpoint.auth_token
        || request.scope != endpoint.scope
    {
        return Err(format!(
            "unexpected handshake: request={request:?}, endpoint={endpoint:?}"
        ));
    }
    write_json_line(
        &mut stream,
        &IpcHandshakeResponse {
            protocol_version: endpoint.protocol_version,
            daemon_version: endpoint.daemon_version.clone(),
            accepted: true,
            rejection_reason: None,
        },
    )?;

    let Some(frame) = read_json_line::<ClientFrame>(&mut reader)? else {
        return Ok(());
    };
    match frame {
        ClientFrame::Subscribe { channels } if channels == ["issue_monitor"] => {
            subscriptions.fetch_add(1, Ordering::AcqRel);
            write_json_line(&mut stream, &DaemonFrame::Ack)?;
            write_json_line(
                &mut stream,
                &DaemonFrame::Event {
                    channel: "issue_monitor".to_string(),
                    payload: event_payload.clone(),
                },
            )?;

            // Keep the stream alive until gwtd's bounded subscription expires.
            // A close immediately after Event would turn the successful run
            // into a transport failure before its wall-clock deadline.
            let mut drain = String::new();
            match reader.read_line(&mut drain) {
                Ok(_) => Ok(()),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    Ok(())
                }
                Err(error) => Err(format!("wait for subscriber close: {error}")),
            }
        }
        other => Err(format!("unexpected client frame: {other:?}")),
    }
}

fn read_json_line<T: DeserializeOwned>(
    reader: &mut BufReader<UnixStream>,
) -> Result<Option<T>, String> {
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .map_err(|error| format!("read frame: {error}"))?;
    if bytes == 0 {
        return Ok(None);
    }
    serde_json::from_str(line.trim_end())
        .map(Some)
        .map_err(|error| format!("parse frame: {error}; line={line:?}"))
}

fn write_json_line<T: serde::Serialize>(stream: &mut UnixStream, value: &T) -> Result<(), String> {
    serde_json::to_writer(&mut *stream, value)
        .map_err(|error| format!("serialize frame: {error}"))?;
    stream
        .write_all(b"\n")
        .map_err(|error| format!("write frame: {error}"))
}

fn canonical(path: &Path) -> PathBuf {
    dunce::canonicalize(path)
        .unwrap_or_else(|error| panic!("canonicalize {}: {error}", path.display()))
}

fn project_store(project_root: &Path, scope: &RuntimeScope, gwt_home: &Path) -> Value {
    json!({
        "project_root": project_root.display().to_string(),
        "hash": scope.repo_hash,
        "source": "path_fallback",
        "identity_resolved": false,
        "store_path": gwt_home.join("projects").join(&scope.repo_hash).display().to_string(),
    })
}

fn issue_monitor_event(project_root: &Path, scope: &RuntimeScope, gwt_home: &Path) -> Value {
    json!({
        "source_pid": 424_242,
        "event": "status",
        "payload": {
            "enabled": true,
            "autonomous_mode": true,
            "max_active_agents": 3,
            "queue_len": 2,
            "active_count": 1,
        },
        "project_store": project_store(project_root, scope, gwt_home),
    })
}

fn subscribe_envelope(project_root: Option<&Path>) -> Value {
    let mut params = json!({
        "channels": ["issue_monitor"],
        "timeout_seconds": 1,
    });
    if let Some(project_root) = project_root {
        params["project_root"] = Value::String(project_root.display().to_string());
    }
    json!({
        "schema_version": 1,
        "operation": "daemon.subscribe",
        "params": params,
    })
}

fn event_frame(output: &CommandOutput) -> DaemonFrame {
    output
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<DaemonFrame>(line).ok())
        .find(|frame| matches!(frame, DaemonFrame::Event { .. }))
        .unwrap_or_else(|| {
            panic!(
                "gwtd output contained no daemon Event; status={}, stdout={}, stderr={}",
                output.status, output.stdout, output.stderr
            )
        })
}

#[test]
fn explicit_project_root_selects_only_the_target_daemon_and_preserves_event_identity() {
    let fixture = Fixture::new();
    let caller_scope = RuntimeScope::from_project_root(&fixture.caller, RuntimeTarget::Host)
        .expect("caller scope");
    let target_scope = RuntimeScope::from_project_root(&fixture.target, RuntimeTarget::Host)
        .expect("target scope");
    assert_ne!(caller_scope, target_scope, "fixture scopes must conflict");

    let caller = FakeDaemon::start(
        &fixture.gwt_home,
        &fixture.caller,
        "caller",
        issue_monitor_event(&fixture.caller, &caller_scope, &fixture.gwt_home),
    );
    let target = FakeDaemon::start(
        &fixture.gwt_home,
        &fixture.target,
        "target",
        issue_monitor_event(&fixture.target, &target_scope, &fixture.gwt_home),
    );

    let output = run_envelope(
        &mut fixture.gwtd(),
        &subscribe_envelope(Some(&fixture.target)),
    );
    assert!(
        output.status.success(),
        "explicit subscribe failed: stdout={}, stderr={}",
        output.stdout,
        output.stderr
    );
    assert_eq!(caller.handshake_count(), 0, "cwd daemon is not authority");
    assert_eq!(caller.subscription_count(), 0);
    assert_eq!(target.handshake_count(), 1, "target handshake only");
    assert_eq!(target.subscription_count(), 1);
    let DaemonFrame::Event { channel, payload } = event_frame(&output) else {
        unreachable!("event_frame returns Event")
    };
    assert_eq!(channel, "issue_monitor");
    assert_eq!(
        payload["project_store"],
        project_store(&fixture.target, &target_scope, &fixture.gwt_home),
        "the wire identity is the event's actual target authority"
    );
    assert_eq!(payload["event"], "status");
    assert_eq!(payload["payload"]["enabled"], true);
    assert_eq!(payload["payload"]["autonomous_mode"], true);
    assert_eq!(payload["payload"]["max_active_agents"], 3);
    assert_eq!(payload["payload"]["queue_len"], 2);
    assert_eq!(payload["payload"]["active_count"], 1);

    // This fake owns only the IPC authority contract; it intentionally does
    // not impersonate `issue.monitor.status`. StatusView -> daemon field
    // semantic parity is covered by the production projection unit tests.
    target.finish();
    caller.finish();
}

#[test]
fn omitted_project_root_keeps_the_caller_cwd_authority() {
    let fixture = Fixture::new();
    let caller_scope = RuntimeScope::from_project_root(&fixture.caller, RuntimeTarget::Host)
        .expect("caller scope");
    let target_scope = RuntimeScope::from_project_root(&fixture.target, RuntimeTarget::Host)
        .expect("target scope");
    let caller = FakeDaemon::start(
        &fixture.gwt_home,
        &fixture.caller,
        "caller",
        issue_monitor_event(&fixture.caller, &caller_scope, &fixture.gwt_home),
    );
    let target = FakeDaemon::start(
        &fixture.gwt_home,
        &fixture.target,
        "target",
        issue_monitor_event(&fixture.target, &target_scope, &fixture.gwt_home),
    );

    let output = run_envelope(&mut fixture.gwtd(), &subscribe_envelope(None));
    assert!(
        output.status.success(),
        "cwd subscribe failed: stdout={}, stderr={}",
        output.stdout,
        output.stderr
    );
    assert_eq!(caller.handshake_count(), 1);
    assert_eq!(caller.subscription_count(), 1);
    assert_eq!(
        target.handshake_count(),
        0,
        "omission must not select target"
    );
    assert_eq!(target.subscription_count(), 0);
    let DaemonFrame::Event { payload, .. } = event_frame(&output) else {
        unreachable!("event_frame returns Event")
    };
    assert_eq!(
        payload["project_store"],
        project_store(&fixture.caller, &caller_scope, &fixture.gwt_home)
    );
    target.finish();
    caller.finish();
}

#[test]
fn invalid_explicit_project_root_fails_without_contacting_the_cwd_daemon() {
    let fixture = Fixture::new();
    let caller_scope = RuntimeScope::from_project_root(&fixture.caller, RuntimeTarget::Host)
        .expect("caller scope");
    let caller = FakeDaemon::start(
        &fixture.gwt_home,
        &fixture.caller,
        "caller",
        issue_monitor_event(&fixture.caller, &caller_scope, &fixture.gwt_home),
    );
    let missing = fixture.target.join("missing");
    let file = fixture.target.join("not-a-directory");
    fs::write(&file, b"fixture").expect("create explicit file root");

    for invalid in [&missing, &file] {
        let output = run_envelope(&mut fixture.gwtd(), &subscribe_envelope(Some(invalid)));
        assert!(
            !output.status.success(),
            "invalid explicit root must fail closed: root={}, stdout={}, stderr={}",
            invalid.display(),
            output.stdout,
            output.stderr
        );
        assert!(
            output.stdout.contains(&invalid.display().to_string()),
            "failure must name the rejected root: root={}, stdout={}, stderr={}",
            invalid.display(),
            output.stdout,
            output.stderr
        );
    }

    assert_eq!(
        caller.handshake_count(),
        0,
        "a rejected explicit authority must never fall back to cwd"
    );
    assert_eq!(caller.subscription_count(), 0);
    caller.finish();
}
