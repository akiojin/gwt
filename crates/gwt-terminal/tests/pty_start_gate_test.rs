use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use gwt_terminal::{
    pty::{run_start_gate_from_env, ProcessPolicy, ProcessPriority, SpawnConfig},
    Pane, PaneStatus, PtyHandle,
};

const HELPER_ROLE_ENV: &str = "GWT_TERMINAL_TEST_START_GATE_HELPER";
const TARGET_SENTINEL_ENV: &str = "GWT_TERMINAL_TEST_START_GATE_TARGET_SENTINEL";
const CRASH_PARENT_READY_ENV: &str = "GWT_TERMINAL_TEST_START_GATE_CRASH_READY";
const PRIORITY_REPORT_ENV: &str = "GWT_TERMINAL_TEST_START_GATE_PRIORITY_REPORT";
const PRIORITY_GRANDCHILD_REPORT_ENV: &str = "GWT_TERMINAL_TEST_START_GATE_PRIORITY_GRANDCHILD";

fn pty_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("PTY start-gate test lock")
}

fn current_test_exe() -> PathBuf {
    std::env::current_exe().expect("current integration test executable")
}

fn exact_test_args(name: &str) -> Vec<String> {
    vec![
        "--exact".to_string(),
        name.to_string(),
        "--nocapture".to_string(),
    ]
}

fn gate_args_prefix() -> Vec<String> {
    exact_test_args("start_gate_helper_process")
}

fn target_config(sentinel: &Path) -> SpawnConfig {
    SpawnConfig {
        command: current_test_exe().display().to_string(),
        args: exact_test_args("start_gate_target_process"),
        cols: 80,
        rows: 24,
        env: HashMap::from([
            (HELPER_ROLE_ENV.to_string(), "1".to_string()),
            (
                TARGET_SENTINEL_ENV.to_string(),
                sentinel.display().to_string(),
            ),
        ]),
        remove_env: Vec::new(),
        cwd: None,
    }
}

fn wait_for_path(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn start_gate_helper_process() {
    if std::env::var(HELPER_ROLE_ENV).ok().as_deref() != Some("1") {
        return;
    }
    match run_start_gate_from_env() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("start-gate helper failed: {error}");
            std::process::exit(90);
        }
    }
}

#[test]
fn start_gate_target_process() {
    let Some(path) = std::env::var_os(TARGET_SENTINEL_ENV) else {
        return;
    };
    for internal in [
        "GWT_INTERNAL_PTY_GATE_ENDPOINT",
        "GWT_INTERNAL_PTY_GATE_NONCE",
        "GWT_INTERNAL_PTY_GATE_TARGET",
    ] {
        assert!(
            std::env::var_os(internal).is_none(),
            "private start-gate key leaked to target: {internal}"
        );
    }
    fs::write(path, b"target executed\n").expect("write target sentinel");
}

#[test]
fn hard_crash_parent_process() {
    let Some(ready_path) = std::env::var_os(CRASH_PARENT_READY_ENV) else {
        return;
    };
    let target_sentinel =
        PathBuf::from(std::env::var_os(TARGET_SENTINEL_ENV).expect("hard-crash target sentinel"));
    let _pending = PtyHandle::spawn_pending(
        target_config(&target_sentinel),
        current_test_exe(),
        gate_args_prefix(),
        "hard-crash-nonce",
    )
    .expect("spawn pending PTY in crash parent");

    let mut ready = fs::File::create(ready_path).expect("create crash-ready sentinel");
    use std::io::Write as _;
    ready.write_all(b"pending handshake complete\n").unwrap();
    ready.sync_all().unwrap();
    std::process::abort();
}

#[test]
fn pending_pty_does_not_run_target_until_release() {
    let _guard = pty_test_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let sentinel = temp.path().join("target-ran");

    let pending = PtyHandle::spawn_pending(
        target_config(&sentinel),
        current_test_exe(),
        gate_args_prefix(),
        "release-nonce",
    )
    .expect("spawn pending PTY");
    assert!(pending.process_id().is_some());
    thread::sleep(Duration::from_millis(200));
    assert!(!sentinel.exists(), "target ran before release");

    let handle = pending.release().expect("release pending PTY");
    assert!(wait_for_path(&sentinel, Duration::from_secs(5)));
    drop(handle);
}

#[test]
fn pending_pty_abort_and_drop_never_run_target() {
    let _guard = pty_test_lock();
    let temp = tempfile::tempdir().expect("tempdir");

    let aborted_sentinel = temp.path().join("aborted-target-ran");
    PtyHandle::spawn_pending(
        target_config(&aborted_sentinel),
        current_test_exe(),
        gate_args_prefix(),
        "abort-nonce",
    )
    .expect("spawn abortable PTY")
    .abort()
    .expect("abort pending PTY");
    thread::sleep(Duration::from_millis(200));
    assert!(!aborted_sentinel.exists(), "aborted target ran");

    let dropped_sentinel = temp.path().join("dropped-target-ran");
    let pending = PtyHandle::spawn_pending(
        target_config(&dropped_sentinel),
        current_test_exe(),
        gate_args_prefix(),
        "drop-nonce",
    )
    .expect("spawn droppable PTY");
    drop(pending);
    thread::sleep(Duration::from_millis(200));
    assert!(!dropped_sentinel.exists(), "dropped target ran");
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the crash harness must execute this exact integration-test binary"
)]
fn parent_hard_crash_before_release_never_runs_target() {
    let _guard = pty_test_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let ready = temp.path().join("parent-ready");
    let sentinel = temp.path().join("target-ran");

    let status = Command::new(current_test_exe())
        .args(exact_test_args("hard_crash_parent_process"))
        .env(CRASH_PARENT_READY_ENV, &ready)
        .env(TARGET_SENTINEL_ENV, &sentinel)
        .status()
        .expect("run hard-crash parent harness");

    assert!(
        !status.success(),
        "hard-crash harness unexpectedly succeeded"
    );
    assert!(ready.exists(), "harness crashed before pending handshake");
    thread::sleep(Duration::from_millis(500));
    assert!(!sentinel.exists(), "target ran after parent hard crash");
}

#[test]
fn pending_pane_materializes_only_after_release() {
    let _guard = pty_test_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let sentinel = temp.path().join("pane-target-ran");

    let pending = Pane::new_pending_with_spawn_config(
        "pending-pane".to_string(),
        target_config(&sentinel),
        current_test_exe(),
        gate_args_prefix(),
        "pane-nonce",
    )
    .expect("spawn pending pane");
    assert_eq!(pending.id(), "pending-pane");
    assert!(pending.process_id().is_some());
    assert!(!sentinel.exists());

    let pane = pending.release().expect("release pending pane");
    assert_eq!(pane.status(), &PaneStatus::Running);
    assert!(wait_for_path(&sentinel, Duration::from_secs(5)));
    drop(pane);
}

/// Report the scheduling priority of the current process in the platform's
/// native unit: the Unix nice value, or the Windows priority class name.
fn current_priority_report() -> String {
    #[cfg(unix)]
    {
        // SAFETY: getpriority has no memory-safety preconditions.
        let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS as _, 0) };
        nice.to_string()
    }
    #[cfg(windows)]
    {
        format!(
            "{:?}",
            gwt_core::process_tree::process_priority_class(std::process::id())
                .expect("query current process priority class")
        )
    }
}

fn expected_priority_report(priority: ProcessPriority) -> String {
    #[cfg(unix)]
    {
        priority.unix_nice().to_string()
    }
    #[cfg(windows)]
    {
        format!("{:?}", priority.windows_priority_class())
    }
}

/// Target role: record this process' priority, then spawn a grandchild with
/// the same role so descendant inheritance is observable from the test.
#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the grandchild must be this exact integration-test binary"
)]
fn start_gate_priority_target_process() {
    let Some(report) = std::env::var_os(PRIORITY_REPORT_ENV) else {
        return;
    };
    fs::write(&report, current_priority_report()).expect("write priority report");
    if let Some(grandchild_report) = std::env::var_os(PRIORITY_GRANDCHILD_REPORT_ENV) {
        let status = Command::new(current_test_exe())
            .args(exact_test_args("start_gate_priority_target_process"))
            .env(PRIORITY_REPORT_ENV, grandchild_report)
            .env_remove(PRIORITY_GRANDCHILD_REPORT_ENV)
            .status()
            .expect("spawn grandchild priority reporter");
        assert!(status.success(), "grandchild priority reporter failed");
    }
}

#[test]
fn process_priority_maps_to_platform_scheduling_values() {
    assert_eq!(ProcessPriority::Normal.unix_nice(), 0);
    assert_eq!(ProcessPriority::BelowNormal.unix_nice(), 10);
    assert_eq!(ProcessPriority::Idle.unix_nice(), 19);
    let policy = ProcessPolicy {
        priority: ProcessPriority::BelowNormal,
        cpu_limit_percent: Some(50),
    };
    assert_eq!(policy.priority, ProcessPriority::BelowNormal);
    assert_eq!(policy.cpu_limit_percent, Some(50));
}

#[test]
fn start_gate_policy_lowers_target_and_grandchild_priority() {
    let _guard = pty_test_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let target_report = temp.path().join("target-priority");
    let grandchild_report = temp.path().join("grandchild-priority");

    let mut config = target_config(&temp.path().join("unused-sentinel"));
    config.args = exact_test_args("start_gate_priority_target_process");
    config.env.insert(
        PRIORITY_REPORT_ENV.to_string(),
        target_report.display().to_string(),
    );
    config.env.insert(
        PRIORITY_GRANDCHILD_REPORT_ENV.to_string(),
        grandchild_report.display().to_string(),
    );

    let pending = PtyHandle::spawn_pending(
        config,
        current_test_exe(),
        gate_args_prefix(),
        "policy-nonce",
    )
    .expect("spawn pending PTY");
    pending
        .apply_policy(ProcessPolicy {
            priority: ProcessPriority::BelowNormal,
            cpu_limit_percent: Some(50),
        })
        .expect("apply resource policy before release");
    assert!(!target_report.exists(), "target ran before release");

    let handle = pending.release().expect("release pending PTY");
    assert!(wait_for_path(&target_report, Duration::from_secs(10)));
    assert!(wait_for_path(&grandchild_report, Duration::from_secs(10)));
    let expected = expected_priority_report(ProcessPriority::BelowNormal);
    // The report file may be observed between create and write; poll briefly.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let target = fs::read_to_string(&target_report).unwrap_or_default();
        let grandchild = fs::read_to_string(&grandchild_report).unwrap_or_default();
        if target.trim() == expected && grandchild.trim() == expected {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "target priority {target:?} / grandchild priority {grandchild:?}, expected {expected:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    drop(handle);
}

/// SPEC #1921 Phase 86 T518: a target that cannot exist must surface as a
/// pre-spawn failure from `spawn_pending`, exactly like the direct PTY route,
/// so launch retry bookkeeping never observes a released-then-dead target.
#[test]
fn pending_pty_reports_a_missing_target_before_release() {
    let _guard = pty_test_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let mut config = target_config(&temp.path().join("unused-sentinel"));
    config.command = temp
        .path()
        .join("definitely-missing-target")
        .display()
        .to_string();

    let error = match PtyHandle::spawn_pending(
        config,
        current_test_exe(),
        gate_args_prefix(),
        "missing-target-nonce",
    ) {
        Ok(_pending) => panic!("a missing target must fail before release"),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("definitely-missing-target"),
        "error must name the missing target: {error}"
    );
}
