#![cfg(unix)]
//! End-to-end guard for Issue #3476: the runtime daemon must come up on a
//! long runtime path.
//!
//! A fresh-home PM session puts `~/.gwt/projects/<repo>/runtime/daemon/`
//! deep inside a temporary directory. The colocated `<worktree>.sock` then
//! overflows `sockaddr_un.sun_path` and `daemon.start` dies with
//! `path must be shorter than SUN_LEN`, leaving `daemon.subscribe` with no
//! endpoint to attach to. These tests drive the real `gwtd` binary over
//! such a fixture: start must bind, status must probe the live daemon, and
//! a bounded subscribe must attach to it.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use gwt_core::{
    daemon::{DAEMON_SOCKET_DIR_ENV, MAX_UNIX_SOCKET_PATH_LEN},
    process::hidden_command,
};
use tempfile::TempDir;

/// Bounded wait for the daemon to publish its endpoint and answer probes.
const READY_TIMEOUT: Duration = Duration::from_secs(30);
/// Bounded wait for a single short-lived `gwtd` invocation.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

const START_ENVELOPE: &str = r#"{"schema_version":1,"operation":"daemon.start","params":{}}"#;
const STATUS_ENVELOPE: &str = r#"{"schema_version":1,"operation":"daemon.status","params":{}}"#;
const SUBSCRIBE_ENVELOPE: &str =
    r#"{"schema_version":1,"operation":"daemon.subscribe","params":{"channels":["board"]}}"#;

/// Kills the child on drop so a failing assertion never leaks a daemon.
struct ChildGuard {
    child: Child,
    label: &'static str,
}

impl ChildGuard {
    fn assert_alive(&mut self, context: &str) {
        if let Ok(Some(status)) = self.child.try_wait() {
            panic!("{} exited ({status}); {context}", self.label);
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A HOME long enough that the colocated daemon socket cannot fit in
/// `sun_path`.
struct LongPathFixture {
    _root: TempDir,
    home: PathBuf,
    project: PathBuf,
    logs: PathBuf,
}

impl LongPathFixture {
    fn new() -> Self {
        let root = TempDir::new().expect("fixture tempdir");
        // `<home>/.gwt/projects/<16>/runtime/daemon/<16>.sock`
        let endpoint_suffix_len =
            "/.gwt/projects/0123456789abcdef/runtime/daemon/".len() + "0123456789abcdef.sock".len();

        let mut home = root.path().to_path_buf();
        while home.as_os_str().len() + endpoint_suffix_len <= MAX_UNIX_SOCKET_PATH_LEN {
            home = home.join("fresh-home-padding");
        }
        let project = home.join("project");
        let logs = root.path().join("logs");
        fs::create_dir_all(&project).expect("create long project");
        fs::create_dir_all(&logs).expect("create log dir");

        Self {
            _root: root,
            home,
            project,
            logs,
        }
    }

    fn gwtd(&self) -> Command {
        let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
        // The surrounding gwt session exports launch context that would
        // otherwise redirect this daemon at the developer's real runtime.
        for key in [
            "GWT_BIN_PATH",
            "GWT_BROWSER_URL_FILE",
            "GWT_HOOK_BIN",
            "GWT_HOOK_FORWARD_TOKEN",
            "GWT_HOOK_FORWARD_URL",
            "GWT_PROJECT_ROOT",
            "GWT_REPO_HASH",
            "GWT_SESSION_ID",
            "GWT_SESSION_KIND",
            "GWT_SESSION_RUNTIME_PATH",
            "GWT_WORKTREE_HASH",
            DAEMON_SOCKET_DIR_ENV,
        ] {
            command.env_remove(key);
        }
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .current_dir(&self.project);
        command
    }

    /// Spawns a long-running `gwtd` envelope with its output teed to files,
    /// so a full pipe can never wedge the daemon and failures stay
    /// debuggable.
    fn spawn_logged(
        &self,
        command: &mut Command,
        envelope: &str,
        label: &'static str,
    ) -> ChildGuard {
        let stdout = fs::File::create(self.logs.join(format!("{label}.out"))).expect("stdout log");
        let stderr = fs::File::create(self.logs.join(format!("{label}.err"))).expect("stderr log");
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .unwrap_or_else(|err| panic!("spawn {label}: {err}"));
        write_envelope(&mut child, envelope);
        ChildGuard { child, label }
    }

    fn log(&self, label: &str) -> String {
        let out = fs::read_to_string(self.logs.join(format!("{label}.out"))).unwrap_or_default();
        let err = fs::read_to_string(self.logs.join(format!("{label}.err"))).unwrap_or_default();
        format!("--- {label} stdout ---\n{out}--- {label} stderr ---\n{err}")
    }
}

fn write_envelope(child: &mut Child, envelope: &str) {
    let mut stdin = child.stdin.take().expect("stdin pipe");
    stdin
        .write_all(envelope.as_bytes())
        .and_then(|()| stdin.write_all(b"\n"))
        .expect("write gwtd envelope");
    // Dropping the handle closes stdin so gwtd stops waiting for more.
}

/// Runs a short-lived `gwtd` envelope under a bounded deadline.
fn run_envelope(command: &mut Command, envelope: &str) -> (String, String) {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gwtd");
    write_envelope(&mut child, envelope);

    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("gwtd envelope {envelope} did not finish within {COMMAND_TIMEOUT:?}");
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(err) => panic!("wait for gwtd failed: {err}"),
        }
    }

    let output = child.wait_with_output().expect("collect gwtd output");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Returns the human-readable payload `gwtd` wrapped in its JSON envelope.
fn daemon_status(fixture: &LongPathFixture) -> String {
    let (stdout, stderr) = run_envelope(&mut fixture.gwtd(), STATUS_ENVELOPE);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|err| {
        panic!("unparsable daemon.status envelope ({err}): {stdout}{stderr}")
    });
    parsed["output"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Polls `daemon.status` until it reports a live, probeable daemon.
fn wait_for_running_status(fixture: &LongPathFixture, daemon: &mut ChildGuard) -> String {
    let deadline = Instant::now() + READY_TIMEOUT;
    let mut last = String::new();
    while Instant::now() < deadline {
        daemon.assert_alive(&format!(
            "it never became reachable. last status: {last}\n{}",
            fixture.log("daemon-start")
        ));
        last = daemon_status(fixture);
        if last.starts_with("running ") && last.contains("probe=ok") {
            return last;
        }
        thread::sleep(POLL_INTERVAL);
    }
    panic!(
        "daemon never reported a probeable status; last status: {last}\n{}",
        fixture.log("daemon-start")
    );
}

fn status_field(status: &str, key: &str) -> String {
    status
        .split_whitespace()
        .find_map(|field| field.strip_prefix(key))
        .unwrap_or_else(|| panic!("status line has no {key} field: {status}"))
        .to_string()
}

#[test]
fn daemon_starts_and_serves_subscribers_on_a_long_runtime_path() {
    let fixture = LongPathFixture::new();
    let mut daemon = fixture.spawn_logged(&mut fixture.gwtd(), START_ENVELOPE, "daemon-start");

    let status = wait_for_running_status(&fixture, &mut daemon);
    let bind = status_field(&status, "bind=");
    assert!(
        bind.len() <= MAX_UNIX_SOCKET_PATH_LEN,
        "advertised bind path is {} bytes, over the {MAX_UNIX_SOCKET_PATH_LEN}-byte limit: {bind}",
        bind.len()
    );
    assert!(
        Path::new(&bind).exists(),
        "advertised bind path does not exist: {bind}"
    );

    // A bounded subscribe must attach to that daemon rather than report
    // "no daemon registered". Success is silent on stdout, so prove the
    // attachment through the daemon's own connection count: `daemon.status`
    // probes over its own connection, so a live subscriber pushes the
    // count past one.
    let mut subscriber =
        fixture.spawn_logged(&mut fixture.gwtd(), SUBSCRIBE_ENVELOPE, "daemon-subscribe");

    let deadline = Instant::now() + READY_TIMEOUT;
    let mut observed = 0usize;
    while Instant::now() < deadline {
        subscriber.assert_alive(&format!(
            "it should have kept streaming from the daemon.\n{}",
            fixture.log("daemon-subscribe")
        ));
        let status = wait_for_running_status(&fixture, &mut daemon);
        observed = status_field(&status, "connections=")
            .parse()
            .unwrap_or_else(|err| panic!("unparsable connection count in {status}: {err}"));
        if observed >= 2 {
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }
    assert!(
        observed >= 2,
        "subscriber never showed up in the daemon connection count (saw {observed})\n{}",
        fixture.log("daemon-subscribe")
    );
}

#[test]
fn daemon_start_reports_a_diagnosable_error_when_no_short_socket_dir_is_usable() {
    // AC-6: when no short endpoint can be secured the operator must get a
    // diagnosis, not a bare `path must be shorter than SUN_LEN`. Force
    // that state by pinning the socket directory override at a path that
    // is itself too long.
    let fixture = LongPathFixture::new();
    let mut unusable = fixture.home.join("unusable-socket-base");
    while unusable.as_os_str().len() <= MAX_UNIX_SOCKET_PATH_LEN {
        unusable = unusable.join("socket-base-padding");
    }
    fs::create_dir_all(&unusable).expect("create unusable socket dir");

    let (stdout, stderr) = run_envelope(
        fixture.gwtd().env(DAEMON_SOCKET_DIR_ENV, &unusable),
        START_ENVELOPE,
    );
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains(DAEMON_SOCKET_DIR_ENV),
        "failure must name the environment lever that fixes it: {combined}"
    );
    assert!(
        combined.contains(&MAX_UNIX_SOCKET_PATH_LEN.to_string()),
        "failure must state the platform limit: {combined}"
    );
    assert!(
        !combined.contains("path must be shorter than SUN_LEN"),
        "failure must not leak the raw bind error: {combined}"
    );
}
