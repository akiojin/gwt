use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use gwt_terminal::{
    pty::{run_start_gate_from_env, ProcessPriority, ProcessResourcePolicy, SpawnConfig},
    Pane, PaneStatus, PtyHandle,
};

const HELPER_ROLE_ENV: &str = "GWT_TERMINAL_TEST_START_GATE_HELPER";
const TARGET_SENTINEL_ENV: &str = "GWT_TERMINAL_TEST_START_GATE_TARGET_SENTINEL";
const CRASH_PARENT_READY_ENV: &str = "GWT_TERMINAL_TEST_START_GATE_CRASH_READY";
#[cfg(unix)]
const PRIORITY_REPORT_ENV: &str = "GWT_TERMINAL_TEST_PRIORITY_REPORT";
#[cfg(unix)]
const PRIORITY_GRANDCHILD_REPORT_ENV: &str = "GWT_TERMINAL_TEST_PRIORITY_GRANDCHILD_REPORT";
#[cfg(unix)]
const LOW_PRIORITY_PARENT_ENV: &str = "GWT_TERMINAL_TEST_LOW_PRIORITY_PARENT";
#[cfg(windows)]
const WINDOWS_PRIORITY_REPORT_ENV: &str = "GWT_TERMINAL_TEST_WINDOWS_PRIORITY_REPORT";
#[cfg(windows)]
const WINDOWS_PRIORITY_GRANDCHILD_REPORT_ENV: &str =
    "GWT_TERMINAL_TEST_WINDOWS_PRIORITY_GRANDCHILD_REPORT";
#[cfg(windows)]
const WINDOWS_LIFETIME_REPORT_ENV: &str = "GWT_TERMINAL_TEST_WINDOWS_LIFETIME_REPORT";
#[cfg(windows)]
const WINDOWS_LIFETIME_GRANDCHILD_REPORT_ENV: &str =
    "GWT_TERMINAL_TEST_WINDOWS_LIFETIME_GRANDCHILD_REPORT";

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
#[allow(
    clippy::disallowed_methods,
    reason = "the target harness must execute this exact integration-test binary"
)]
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
    #[cfg(unix)]
    if let Some(report) = std::env::var_os(PRIORITY_REPORT_ENV) {
        let target_nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
        fs::write(&report, format!("target={target_nice}\n")).expect("write target priority");
        let status = Command::new(current_test_exe())
            .args(exact_test_args("start_gate_priority_grandchild_process"))
            .env(PRIORITY_GRANDCHILD_REPORT_ENV, &report)
            .status()
            .expect("spawn priority grandchild");
        assert!(status.success(), "priority grandchild failed: {status}");
        return;
    }
    #[cfg(windows)]
    if let Some(report) = std::env::var_os(WINDOWS_LIFETIME_REPORT_ENV) {
        fs::write(&report, format!("target={}\n", std::process::id()))
            .expect("write Windows target lifetime report");
        let _grandchild = Command::new(current_test_exe())
            .args(exact_test_args(
                "start_gate_windows_lifetime_grandchild_process",
            ))
            .env(WINDOWS_LIFETIME_GRANDCHILD_REPORT_ENV, &report)
            .spawn()
            .expect("spawn Windows lifetime grandchild");
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }
    #[cfg(windows)]
    if let Some(report) = std::env::var_os(WINDOWS_PRIORITY_REPORT_ENV) {
        let (priority, in_job) = windows_process_state(std::process::id());
        fs::write(&report, format!("target={priority},{in_job}\n"))
            .expect("write Windows target policy");
        let status = Command::new(current_test_exe())
            .args(exact_test_args(
                "start_gate_windows_priority_grandchild_process",
            ))
            .env(WINDOWS_PRIORITY_GRANDCHILD_REPORT_ENV, &report)
            .status()
            .expect("spawn Windows priority grandchild");
        assert!(
            status.success(),
            "Windows priority grandchild failed: {status}"
        );
        return;
    }
    fs::write(path, b"target executed\n").expect("write target sentinel");
}

#[cfg(unix)]
#[test]
fn start_gate_priority_grandchild_process() {
    let Some(report) = std::env::var_os(PRIORITY_GRANDCHILD_REPORT_ENV) else {
        return;
    };
    let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(report)
        .expect("open priority report");
    writeln!(file, "grandchild={nice}").expect("append grandchild priority");
}

#[cfg(windows)]
#[test]
fn start_gate_windows_priority_grandchild_process() {
    let Some(report) = std::env::var_os(WINDOWS_PRIORITY_GRANDCHILD_REPORT_ENV) else {
        return;
    };
    let (priority, in_job) = windows_process_state(std::process::id());
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(report)
        .expect("open Windows priority report");
    writeln!(file, "grandchild={priority},{in_job}").expect("append Windows grandchild policy");
}

#[cfg(windows)]
#[test]
fn start_gate_windows_lifetime_grandchild_process() {
    let Some(report) = std::env::var_os(WINDOWS_LIFETIME_GRANDCHILD_REPORT_ENV) else {
        return;
    };
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(report)
        .expect("open Windows lifetime report");
    writeln!(file, "grandchild={}", std::process::id())
        .expect("append Windows grandchild lifetime report");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(windows)]
fn windows_process_state(pid: u32) -> (u32, bool) {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::{
            JobObjects::IsProcessInJob,
            Threading::{GetPriorityClass, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        },
    };

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .expect("open process for policy inspection");
    let priority = unsafe { GetPriorityClass(process) };
    assert_ne!(priority, 0, "GetPriorityClass failed for {pid}");
    let mut in_job = windows::core::BOOL::default();
    unsafe { IsProcessInJob(process, None, &mut in_job) }.expect("inspect process Job membership");
    unsafe { CloseHandle(process) }.expect("close policy inspection handle");
    (priority, in_job.as_bool())
}

#[cfg(windows)]
fn reported_windows_pid(content: &str, key: &str) -> u32 {
    content
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_else(|| panic!("missing {key} PID in {content:?}"))
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
fn pending_pty_release_reports_target_spawn_failure() {
    let _guard = pty_test_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let missing_target = temp.path().join("missing-agent-target");
    let config = SpawnConfig {
        command: missing_target.display().to_string(),
        args: Vec::new(),
        cols: 80,
        rows: 24,
        env: HashMap::from([(HELPER_ROLE_ENV.to_string(), "1".to_string())]),
        remove_env: Vec::new(),
        cwd: None,
    };

    let pending = PtyHandle::spawn_pending(
        config,
        current_test_exe(),
        gate_args_prefix(),
        "missing-target-nonce",
    )
    .expect("spawn pending PTY");
    let error = match pending.release() {
        Ok(handle) => {
            drop(handle);
            panic!("release must report a target spawn failure")
        }
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("execute released PTY target"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn pending_policy_is_inherited_by_target_and_grandchild() {
    let _guard = pty_test_lock();
    if std::env::var_os(LOW_PRIORITY_PARENT_ENV).is_some() {
        assert_eq!(
            unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 15) },
            0,
            "lower test-parent priority"
        );
    }
    let parent_nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
    let expected_nice = parent_nice.max(10);
    let temp = tempfile::tempdir().expect("tempdir");
    let report = temp.path().join("priority-report");
    let mut config = target_config(&report);
    config.env.insert(
        PRIORITY_REPORT_ENV.to_string(),
        report.display().to_string(),
    );
    let resource_policy = ProcessResourcePolicy {
        priority: ProcessPriority::BelowNormal,
        cpu_limit_percent: None,
    };

    let pending = PtyHandle::spawn_pending_with_resource_policy(
        config,
        current_test_exe(),
        gate_args_prefix(),
        "priority-nonce",
        resource_policy,
    )
    .expect("spawn policy-gated PTY");
    assert!(!report.exists(), "target ran before policy gate release");
    let gate_pid = pending.process_id().expect("gated helper pid");
    let gate_nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, gate_pid) };
    assert_eq!(gate_nice, expected_nice, "policy must apply before release");
    let handle = pending.release().expect("release policy-gated PTY");
    assert!(wait_for_path(&report, Duration::from_secs(5)));
    let deadline = Instant::now() + Duration::from_secs(5);
    let content = loop {
        let content = fs::read_to_string(&report).unwrap_or_default();
        if content.contains("grandchild=") || Instant::now() >= deadline {
            break content;
        }
        thread::sleep(Duration::from_millis(10));
    };
    assert!(
        content.contains(&format!("target={expected_nice}")),
        "{content}"
    );
    assert!(
        content.contains(&format!("grandchild={expected_nice}")),
        "{content}"
    );
    drop(handle);
}

#[cfg(unix)]
#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "the priority harness must execute this exact integration-test binary"
)]
fn pending_policy_never_raises_an_already_lower_priority_parent() {
    let status = Command::new(current_test_exe())
        .args(exact_test_args(
            "pending_policy_is_inherited_by_target_and_grandchild",
        ))
        .env(LOW_PRIORITY_PARENT_ENV, "1")
        .status()
        .expect("run inheritance test below nice 10");
    assert!(
        status.success(),
        "lower-priority inheritance failed: {status}"
    );
}

#[cfg(windows)]
#[test]
fn pending_windows_policy_is_inherited_by_target_and_grandchild() {
    use windows::Win32::System::Threading::BELOW_NORMAL_PRIORITY_CLASS;

    let _guard = pty_test_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let report = temp.path().join("windows-priority-report");
    let mut config = target_config(&report);
    config.env.insert(
        WINDOWS_PRIORITY_REPORT_ENV.to_string(),
        report.display().to_string(),
    );
    let pending = PtyHandle::spawn_pending_with_resource_policy(
        config,
        current_test_exe(),
        gate_args_prefix(),
        "windows-priority-nonce",
        ProcessResourcePolicy {
            priority: ProcessPriority::BelowNormal,
            cpu_limit_percent: Some(33),
        },
    )
    .expect("spawn Windows policy-gated PTY");
    assert!(!report.exists(), "target ran before Windows policy release");
    let gate_pid = pending.process_id().expect("Windows gate helper pid");
    assert_eq!(
        windows_process_state(gate_pid),
        (BELOW_NORMAL_PRIORITY_CLASS.0, true),
        "gate must be lower priority and Job-owned before release"
    );
    let handle = pending.release().expect("release Windows policy-gated PTY");
    assert!(wait_for_path(&report, Duration::from_secs(5)));
    let deadline = Instant::now() + Duration::from_secs(5);
    let content = loop {
        let content = fs::read_to_string(&report).unwrap_or_default();
        if content.contains("grandchild=") || Instant::now() >= deadline {
            break content;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let expected = format!("{},true", BELOW_NORMAL_PRIORITY_CLASS.0);
    assert!(content.contains(&format!("target={expected}")), "{content}");
    assert!(
        content.contains(&format!("grandchild={expected}")),
        "{content}"
    );
    drop(handle);
}

#[cfg(windows)]
#[test]
fn pending_windows_job_drop_terminates_target_and_grandchild() {
    use windows::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{
            OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
            PROCESS_TERMINATE,
        },
    };

    let _guard = pty_test_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let report = temp.path().join("windows-lifetime-report");
    let mut config = target_config(&report);
    config.env.insert(
        WINDOWS_LIFETIME_REPORT_ENV.to_string(),
        report.display().to_string(),
    );
    let pending = PtyHandle::spawn_pending_with_resource_policy(
        config,
        current_test_exe(),
        gate_args_prefix(),
        "windows-lifetime-nonce",
        ProcessResourcePolicy {
            priority: ProcessPriority::BelowNormal,
            cpu_limit_percent: Some(33),
        },
    )
    .expect("spawn Windows lifetime policy-gated PTY");
    let handle = pending.release().expect("release Windows lifetime PTY");
    let deadline = Instant::now() + Duration::from_secs(5);
    let content = loop {
        let content = fs::read_to_string(&report).unwrap_or_default();
        if content.contains("target=") && content.contains("grandchild=") {
            break content;
        }
        assert!(
            Instant::now() < deadline,
            "Windows target/grandchild did not report live PIDs: {content:?}"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let target_pid = reported_windows_pid(&content, "target=");
    let grandchild_pid = reported_windows_pid(&content, "grandchild=");
    let access = PROCESS_SYNCHRONIZE | PROCESS_TERMINATE;
    let target_process =
        unsafe { OpenProcess(access, false, target_pid) }.expect("open Windows target wait handle");
    let grandchild_process = unsafe { OpenProcess(access, false, grandchild_pid) }
        .expect("open Windows grandchild wait handle");

    drop(handle);
    let target_wait = unsafe { WaitForSingleObject(target_process, 5_000) };
    let grandchild_wait = unsafe { WaitForSingleObject(grandchild_process, 5_000) };
    if target_wait != WAIT_OBJECT_0 {
        let _ = unsafe { TerminateProcess(target_process, 91) };
    }
    if grandchild_wait != WAIT_OBJECT_0 {
        let _ = unsafe { TerminateProcess(grandchild_process, 92) };
    }
    unsafe { CloseHandle(target_process) }.expect("close Windows target wait handle");
    unsafe { CloseHandle(grandchild_process) }.expect("close Windows grandchild wait handle");
    assert_eq!(target_wait, WAIT_OBJECT_0, "Job close did not kill target");
    assert_eq!(
        grandchild_wait, WAIT_OBJECT_0,
        "Job close did not kill target descendant"
    );
}

#[cfg(windows)]
#[test]
fn invalid_windows_cpu_policy_never_releases_target() {
    let _guard = pty_test_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let sentinel = temp.path().join("invalid-policy-target-ran");
    let error = match PtyHandle::spawn_pending_with_resource_policy(
        target_config(&sentinel),
        current_test_exe(),
        gate_args_prefix(),
        "invalid-windows-policy-nonce",
        ProcessResourcePolicy {
            priority: ProcessPriority::BelowNormal,
            cpu_limit_percent: Some(0),
        },
    ) {
        Ok(pending) => {
            let _ = pending.abort();
            panic!("invalid CPU policy must fail closed");
        }
        Err(error) => error,
    };
    assert!(error.to_string().contains("CPU rate"), "{error}");
    thread::sleep(Duration::from_millis(200));
    assert!(!sentinel.exists(), "invalid policy released the target");
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
