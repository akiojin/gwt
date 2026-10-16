//! The real daemon must persist scan timings even without a GUI subscriber.

use std::{
    fs,
    io::Write,
    process::{Child, Stdio},
    thread,
    time::{Duration, Instant},
};

use gwt_core::{
    daemon::{RuntimeScope, RuntimeTarget},
    logging::current_log_file,
    process::hidden_command,
};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn issue_5644_daemon_start_persists_scan_stage_and_total_timings() {
    let fixture = tempfile::tempdir().expect("fixture");
    let home = fixture.path().join("home");
    let project = fixture.path().join("project");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&project).unwrap();
    let project = dunce::canonicalize(project).unwrap();
    let scope = RuntimeScope::from_project_root(&project, RuntimeTarget::Host).unwrap();
    let prefs_path = home
        .join(".gwt/projects")
        .join(scope.repo_hash)
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
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start daemon"),
    );
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
        assert!(
            daemon.0.try_wait().unwrap().is_none(),
            "daemon exited before persisting scan timings"
        );
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
        assert!(
            Instant::now() < deadline,
            "daemon did not persist local-stage and total timings in {}: {contents}",
            log.display()
        );
        thread::sleep(Duration::from_millis(100));
    }
}
