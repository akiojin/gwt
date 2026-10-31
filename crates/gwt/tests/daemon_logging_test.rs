//! The real daemon must persist scan timings even without a GUI subscriber.

use std::{
    fs,
    io::{Read, Write},
    process::{Child, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use gwt_core::{
    daemon::{ClientFrame, DaemonEndpoint, DaemonFrame, RuntimeScope, RuntimeTarget},
    logging::current_log_file,
    process::hidden_command,
};

struct DiagnosticCapture {
    prefix: Arc<Mutex<Vec<u8>>>,
    reader: thread::JoinHandle<bool>,
}

struct Daemon(Child, Vec<DiagnosticCapture>);

impl Daemon {
    fn diagnostic(
        &mut self,
        status: Option<ExitStatus>,
        log_exists: bool,
        stages: [Option<bool>; 3],
    ) -> String {
        let captures: Vec<_> = self
            .1
            .drain(..)
            .map(|capture| {
                // Descendants can retain a pipe after the direct child exits.
                // Never extend the fixture's deadline waiting for their EOF.
                let complete = if capture.reader.is_finished() {
                    capture.reader.join().unwrap_or(false)
                } else {
                    false
                };
                match capture.prefix.lock() {
                    Ok(prefix) => serde_json::json!({
                        "complete": complete,
                        "prefix_limit_reached": prefix.len() == 16 * 1024,
                        "flags": diagnostic_output_flags(&prefix),
                    }),
                    Err(_) => serde_json::json!({"complete": false}),
                }
            })
            .collect();
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt;
            status.and_then(|status| status.signal())
        };
        #[cfg(not(unix))]
        let signal: Option<i32> = None;
        let summary = serde_json::json!({
            "exit_code": status.and_then(|status| status.code()),
            "signal": signal,
            "log_exists": log_exists,
            "scan_started": stages[0],
            "capacity_refresh": stages[1],
            "total": stages[2],
            "stdout_stderr": captures,
        });
        // Unix NamedTempFile creates 0600 files. Windows retains the same safe
        // assertion summary without claiming an owner-only artifact ACL.
        // Never print a persistence error containing its private path.
        #[cfg(unix)]
        if let Ok(mut artifact) = tempfile::Builder::new()
            .prefix("gwt-daemon-start-diagnostic-")
            .suffix(".json")
            .tempfile()
        {
            if serde_json::to_writer(artifact.as_file_mut(), &summary).is_ok() {
                let _ = artifact.keep();
            }
        }
        summary.to_string()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn daemon_diagnostics_discard_secrets_and_drain_after_capture_limit() {
    use std::io::Cursor;

    let secret = "unmarked-private-canary-6576 /private/canary https://private.invalid";
    let mut bytes = b"tokio runtime build failed: ".to_vec();
    bytes.extend(secret.as_bytes());
    bytes.resize(128 * 1024, b'x');
    let mut input = Cursor::new(bytes);
    let prefix = Mutex::new(Vec::new());
    assert!(drain_diagnostic_output(&mut input, &prefix).is_ok());
    assert_eq!(input.position(), 128 * 1024, "must drain past the RAM cap");
    let prefix = prefix.lock().unwrap();
    assert_eq!(prefix.len(), 16 * 1024);
    let summary = diagnostic_output_flags(&prefix);
    assert_eq!(summary["runtime_build_failed"], true);
    assert!(!summary.to_string().contains(secret), "raw capture escaped");
}

fn drain_diagnostic_output(input: &mut impl Read, prefix: &Mutex<Vec<u8>>) -> std::io::Result<()> {
    let mut buffer = [0; 4096];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                let mut prefix = prefix
                    .lock()
                    .map_err(|_| std::io::Error::other("capture unavailable"))?;
                let retained = count.min((16 * 1024usize).saturating_sub(prefix.len()));
                prefix.extend_from_slice(&buffer[..retained]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn diagnostic_output_flags(prefix: &[u8]) -> serde_json::Value {
    let contains = |needle: &str| {
        prefix
            .windows(needle.len())
            .any(|bytes| bytes == needle.as_bytes())
    };
    serde_json::json!({
        "runtime_build_failed": contains("tokio runtime build failed:"),
        "logging_unavailable": contains("gwtd daemon: persistent logging unavailable:"),
        "socket_parent_failed": contains("failed to prepare daemon socket directory:"),
        "lifetime_lease_failed": contains("cannot acquire project daemon lifetime lease:"),
        "socket_bind_failed": contains("failed to bind daemon socket"),
        "endpoint_persist_failed": contains("failed to persist daemon endpoint:"),
        "panicked": contains("panicked at"),
    })
}

fn capture_diagnostic_output(mut input: impl Read + Send + 'static) -> DiagnosticCapture {
    let prefix = Arc::new(Mutex::new(Vec::new()));
    let target = Arc::clone(&prefix);
    let reader = thread::spawn(move || drain_diagnostic_output(&mut input, &target).is_ok());
    DiagnosticCapture { prefix, reader }
}

#[test]
fn issue_5644_daemon_start_persists_scan_stage_and_total_timings() {
    let fixture = gwt_core::test_support::socket_tempdir().expect("fixture");
    let home = fixture.path().join("home");
    let project = fixture.path().join("project");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&project).unwrap();
    let project = dunce::canonicalize(project).unwrap();
    let scope = RuntimeScope::from_project_root(&project, RuntimeTarget::Host).unwrap();
    let prefs_path = home
        .join(".gwt/projects")
        .join(&scope.repo_hash)
        .join("project-state/issue-monitor.json");
    fs::create_dir_all(prefs_path.parent().unwrap()).unwrap();
    let prefs = gwt::IssueMonitorPrefs {
        enabled: true,
        ..Default::default()
    };
    fs::write(&prefs_path, serde_json::to_vec(&prefs).unwrap()).unwrap();

    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
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
    let mut daemon = Daemon(
        command
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("RUST_LOG", "gwt.issue_monitor.scan=info")
            .current_dir(&project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start daemon"),
        Vec::new(),
    );
    daemon
        .1
        .push(capture_diagnostic_output(daemon.0.stdout.take().unwrap()));
    daemon
        .1
        .push(capture_diagnostic_output(daemon.0.stderr.take().unwrap()));
    daemon
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"schema_version":1,"operation":"daemon.start","params":{}}"#)
        .unwrap();

    // A non-repository fixture fails remote resolution locally, exercising a
    // real scan and its early-return timings without reaching GitHub.
    let log = current_log_file(&home.join(".gwt/logs"));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = daemon.0.try_wait().unwrap() {
            let diagnostic = daemon.diagnostic(Some(status), log.is_file(), [None; 3]);
            panic!("daemon exited before persisting scan timings: {diagnostic}");
        }
        let contents = fs::read_to_string(&log).unwrap_or_default();
        let records: Vec<serde_json::Value> = contents
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .filter(|record: &serde_json::Value| record["target"] == "gwt.issue_monitor.scan")
            .collect();
        let has_timing = |stage| {
            records.iter().any(|record| {
                record["fields"]["stage"] == stage && record["fields"]["duration_ms"].is_number()
            })
        };
        let has_started = records
            .iter()
            .any(|record| record["fields"]["state"] == "started");
        if has_started && has_timing("capacity-refresh") && has_timing("total") {
            break;
        }
        // test-hygiene: allow-wall-clock-deadline Bound a real daemon startup hang; scan records determine success.
        if Instant::now() >= deadline {
            let diagnostic = daemon.diagnostic(
                None,
                log.is_file(),
                [
                    Some(has_started),
                    Some(has_timing("capacity-refresh")),
                    Some(has_timing("total")),
                ],
            );
            panic!("daemon did not persist local-stage and total timings: {diagnostic}");
        }
        thread::sleep(Duration::from_millis(100));
    }

    // Issue #5688: the first timer tick used to nest a synchronous Git runtime
    // inside the daemon runtime. A running PID alone does not prove that its
    // Monitor worker survived; request the same atomic projection as the CLI.
    let endpoint: DaemonEndpoint = serde_json::from_slice(
        &fs::read(scope.endpoint_path(&home.join(".gwt"))).expect("daemon endpoint"),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let response = runtime.block_on(async {
        // test-hygiene: allow-wall-clock-deadline Bound a real daemon IPC hang; the status projection determines the verdict.
        tokio::time::timeout(Duration::from_secs(30), async {
            let mut client = gwt::cli::daemon::client::DaemonClient::connect(&endpoint)
                .await
                .expect("daemon accepts status after its first scan");
            client.send_frame(&ClientFrame::Status).await.unwrap();
            client.read_frame::<DaemonFrame>().await.unwrap()
        })
        .await
        .expect("daemon status is bounded")
    });
    let DaemonFrame::Status(status) = response else {
        panic!("expected status, got {response:?}");
    };
    assert!(
        status.issue_monitor.is_some(),
        "Monitor worker publishes status"
    );
}
