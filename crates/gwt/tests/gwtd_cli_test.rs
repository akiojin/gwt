use std::{
    collections::BTreeSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
};

use chrono::Utc;
use gwt_agent::{AgentId, Session};
use gwt_core::process::hidden_command;
use gwt_core::{
    paths::project_scope_hash,
    repo_hash::{compute_path_hash, compute_repo_hash},
    workspace_projection::{
        append_workspace_work_event_to_path, load_workspace_projection_from_path,
        save_workspace_projection_to_path, save_workspace_work_items_projection_to_path, WorkEvent,
        WorkEventKind, WorkItemsProjection, WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary,
        WorkspaceExecutionContainerRef, WorkspaceProjection, WorkspaceStatusCategory,
    },
};
use tempfile::TempDir;

fn isolated_gwtd_command() -> std::process::Command {
    // The application child is not a Cargo test executable. Isolate its
    // startup perf/error-ledger writes as well as the test's explicit files.
    static HOME: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    let home = HOME.get_or_init(|| tempfile::tempdir().expect("child home"));
    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
    command
        .env("HOME", home.path())
        .env("USERPROFILE", home.path());
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
    ] {
        command.env_remove(key);
    }
    command
}

#[test]
fn auto_feedback_real_worker_has_no_consent_auth_process_or_raw_sinks() {
    use gwt_core::test_support::ScopedGwtHome;
    use std::time::{Duration, Instant};
    // Invalid private-role frames must return before ordinary CLI startup,
    // including on platforms where the Windows-only roles are unavailable.
    for role in ["--gwt-feedback-supervisor", "--gwt-feedback-sender"] {
        let empty_home = tempfile::tempdir().unwrap();
        let output = isolated_gwtd_command()
            .args([role])
            .current_dir(empty_home.path())
            .env("HOME", empty_home.path())
            .env("USERPROFILE", empty_home.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert_eq!(fs::read_dir(empty_home.path()).unwrap().count(), 0);
    }
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let _home = ScopedGwtHome::set(home.path());
    init_gwt_origin_repo(repo.path());
    gwt::issue_monitor::save_issue_monitor_prefs(
        &gwt::issue_monitor::issue_monitor_prefs_path_for_repo_path(repo.path()),
        &gwt::issue_monitor::IssueMonitorPrefs {
            autonomous_mode: true,
            ..Default::default()
        },
    )
    .unwrap();
    let bin = home.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let marker = home.path().join("forbidden-gh-call");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let fake = bin.join("gh");
        fs::write(
            &fake,
            format!("#!/bin/sh\n: > '{}'\nexit 1\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(fake, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    fs::write(
        bin.join("gh.cmd"),
        format!(
            "@echo off\r\necho called > \"{}\"\r\nexit /b 1\r\n",
            marker.display()
        ),
    )
    .unwrap();
    let path = env::join_paths(
        std::iter::once(bin).chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
    )
    .unwrap();
    let secret = "unknown-autofeedback-confidential-phrase";
    let mut input = serde_json::json!({"schema_version":1,"operation":"feedback.create","params":{
        "title":secret,"kind":"bug","operation":"workspace.update","refusal":secret,
        "reproduction":secret,"workaround":secret,"body":secret,"labels":[secret],"error_record_id":secret,
    }});
    let upstream_lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(home.path().join(".gwt/feedback-upstream.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&upstream_lock).unwrap();
    let mut command = isolated_gwtd_command();
    command
        .current_dir(repo.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("PATH", path)
        .env("GWT_TEST_GH_SANDBOX", "1")
        .env("GWT_OWNER_GITHUB_TOKEN", "untrusted-override")
        .env("GH_TOKEN", "dummy-never-sent")
        .env_remove("GITHUB_TOKEN")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    fn intake(
        command: &mut std::process::Command,
        input: &serde_json::Value,
        secret: &str,
        expected_state: &str,
    ) -> (serde_json::Value, u32) {
        let mut child = command.spawn().unwrap();
        let pid = child.id();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
        // test-hygiene: allow-wall-clock-deadline Bound a real child process while upstream admission stays locked.
        let deadline = Instant::now() + Duration::from_secs(30);
        while child.try_wait().unwrap().is_none() {
            // test-hygiene: allow-wall-clock-deadline Detect a hung native intake; this is not a latency verdict.
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("intake waited on the sender's held upstream lock");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!String::from_utf8_lossy(&result.stdout).contains(secret));
        assert!(!String::from_utf8_lossy(&result.stderr).contains(secret));
        let envelope: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        let receipt: serde_json::Value =
            serde_json::from_str(envelope["output"].as_str().unwrap()).unwrap();
        assert_eq!(
            receipt["state"], expected_state,
            "source returns its safe intake decision without waiting for a worker"
        );
        (receipt, pid)
    }
    let (receipt, _source_pid) = intake(&mut command, &input, secret, "queued");
    let receipt_path = Path::new(receipt["receipt"].as_str().unwrap());
    assert!(receipt_path.starts_with(home.path()));
    // test-hygiene: allow-wall-clock-deadline Bound observation of the actual detached worker, not simulated app time.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(receipt_path).unwrap()).unwrap();
        let pids = native_feedback_worker_pids(repo.path());
        #[cfg(windows)]
        assert_no_native_feedback_worker_windows(&pids);
        if stored["state"] == "sending" && !pids.is_empty() {
            #[cfg(windows)]
            {
                let (supervisors, senders) = native_feedback_role_counts(&pids);
                assert!(
                    supervisors <= 1 && senders <= 1,
                    "duplicate private worker role"
                );
                if (supervisors, senders) != (1, 1) {
                    assert!(
                        // test-hygiene: allow-wall-clock-deadline Keep the existing native startup bound while observing both private roles.
                        Instant::now() < deadline,
                        "owned worker roles did not settle: {stored}"
                    );
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                assert_eq!(
                    pids.len(),
                    2,
                    "unexpected process beside supervisor and sender"
                );
                assert_eq!(native_owned_attempt(&stored)["phase"], "pre_dispatch");
            }
            #[cfg(unix)]
            assert_eq!(
                pids.len(),
                1,
                "expected one worker process after source {_source_pid}: {pids:?}"
            );
            #[cfg(unix)]
            for pid in pids {
                let group = unsafe { libc::getpgid(pid as libc::pid_t) };
                assert_eq!(
                    group, pid as libc::pid_t,
                    "worker {pid} must lead its own group (source {_source_pid})"
                );
                assert_ne!(group, _source_pid as libc::pid_t);
            }
            break;
        }
        assert!(
            // test-hygiene: allow-wall-clock-deadline Bound real worker startup while its upstream lock is deliberately held.
            Instant::now() < deadline,
            "detached worker did not settle: {stored}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // The actual worker is alive while intake has returned. Revoke policy
    // before releasing admission: no HTTP request or consent fallback follows.
    gwt::issue_monitor::save_issue_monitor_prefs(
        &gwt::issue_monitor::issue_monitor_prefs_path_for_repo_path(repo.path()),
        &gwt::issue_monitor::IssueMonitorPrefs::default(),
    )
    .unwrap();
    drop(upstream_lock);
    loop {
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(receipt_path).unwrap()).unwrap();
        if stored["state"] == "withheld" {
            assert_eq!(stored["error"], "policy_revoked");
            #[cfg(windows)]
            if native_owned_cleanup_confirmed(&stored) {
                break;
            }
            #[cfg(not(windows))]
            break;
        }
        assert!(
            // test-hygiene: allow-wall-clock-deadline Bound actual worker settlement after policy revocation and lock release.
            Instant::now() < deadline,
            "worker did not withhold revoked snapshot: {stored}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    gwt::issue_monitor::save_issue_monitor_prefs(
        &gwt::issue_monitor::issue_monitor_prefs_path_for_repo_path(repo.path()),
        &gwt::issue_monitor::IssueMonitorPrefs {
            autonomous_mode: true,
            ..Default::default()
        },
    )
    .unwrap();
    command.env_remove("GH_TOKEN");
    input["params"]["operation"] = serde_json::json!("verify.run");
    let (no_auth, _) = intake(&mut command, &input, secret, "queued");
    // test-hygiene: allow-wall-clock-deadline Bound the real worker's missing-auth result without any credential interaction.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(no_auth["receipt"].as_str().unwrap()).unwrap())
                .unwrap();
        if stored["state"] == "failed" {
            assert_eq!(stored["error"], "auth_unavailable");
            #[cfg(windows)]
            if native_owned_cleanup_confirmed(&stored) {
                break;
            }
            #[cfg(not(windows))]
            break;
        }
        assert!(
            // test-hygiene: allow-wall-clock-deadline Bound observation of the actual child's safe terminal receipt.
            Instant::now() < deadline,
            "worker did not record missing auth: {stored}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    #[cfg(windows)]
    {
        fn reconcile_unknown(
            command: &mut std::process::Command,
            receipt: &serde_json::Value,
            prior: &serde_json::Value,
            repo: &Path,
            secret: &str,
        ) {
            let status = serde_json::json!({"schema_version":1,"operation":"feedback.status","params":{
                "id": receipt["request_id"],
            }});
            intake(command, &status, secret, "uncertain");
            let path = Path::new(receipt["receipt"].as_str().unwrap());
            let stored: serde_json::Value =
                serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            let attempt = native_owned_attempt(&stored);
            assert_eq!(attempt["id"], prior["owned"]["attempt"]["id"]);
            assert_eq!(attempt["phase"], prior["owned"]["attempt"]["phase"]);
            assert_eq!(attempt["exit"], "unknown");
            assert_eq!(attempt["cleanup"], "unknown");
            assert_eq!(attempt["reason"], "owner_unavailable");
            assert!(stored["issue_number"].is_null());
            native_reject_feedback_retry(command, receipt["request_id"].as_str().unwrap(), secret);
            let after_retry: serde_json::Value =
                serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            assert_eq!(after_retry["owned"], stored["owned"]);
            assert_eq!(after_retry["state"], "uncertain");
            assert!(after_retry["issue_number"].is_null());
            // Exact retained handles already proved exit. This enumeration
            // checks only that status/retry did not create another worker.
            assert!(native_feedback_worker_pids(repo).is_empty());
        }
        command.env("GH_TOKEN", "dummy-never-sent");

        for (operation, commit) in [("build.start", false), ("build.complete", true)] {
            let upstream_lock = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(home.path().join(".gwt/feedback-upstream.lock"))
                .unwrap();
            fs2::FileExt::lock_exclusive(&upstream_lock).unwrap();
            input["params"]["operation"] = serde_json::json!(operation);
            let (crashed, _) = intake(&mut command, &input, secret, "queued");
            let path = Path::new(crashed["receipt"].as_str().unwrap());
            let (mut prior, supervisor, sender) = native_owned_worker_handles(path, repo.path());
            if commit {
                // DISPATCH-COMMITTED kill: exercise the durable local phase,
                // never claim an actual HTTP POST, authentication or Sent.
                let store =
                    gwt_core::feedback::FeedbackStore::new(path.parent().unwrap().to_path_buf());
                store
                    .commit_owned_attempt(
                        crashed["request_id"].as_str().unwrap(),
                        uuid::Uuid::parse_str(native_owned_attempt(&prior)["id"].as_str().unwrap())
                            .unwrap(),
                        || Ok(()),
                    )
                    .unwrap();
                prior = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                assert_eq!(native_owned_attempt(&prior)["phase"], "dispatch_committed");
            }
            supervisor.terminate();
            // test-hygiene: allow-wall-clock-deadline Bound exact native process exit after the fixture kills its proven supervisor.
            let stopped_by = Instant::now() + Duration::from_secs(30);
            loop {
                assert_no_native_feedback_worker_windows(&native_feedback_worker_pids(repo.path()));
                if supervisor.exited() && sender.exited() {
                    break;
                }
                assert!(
                    // test-hygiene: allow-wall-clock-deadline Wait on retained process objects, never infer exit from process-list disappearance.
                    Instant::now() < stopped_by,
                    "supervisor crash left its exact sender alive"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
            reconcile_unknown(&mut command, &crashed, &prior, repo.path(), secret);
            drop(upstream_lock);
        }

        // Block the real cancellation/final-save gate through Dexit. The
        // driver must still stop its exact Job/Child and bound its own life.
        let upstream_lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(home.path().join(".gwt/feedback-upstream.lock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&upstream_lock).unwrap();
        input["params"]["operation"] = serde_json::json!("execution.status");
        let started = Instant::now();
        let (expired, _) = intake(&mut command, &input, secret, "queued");
        let expired_path = Path::new(expired["receipt"].as_str().unwrap());
        let (prior, supervisor, sender) = native_owned_worker_handles(expired_path, repo.path());
        let dispatch_lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(expired_path.parent().unwrap().join(format!(
                "{}.dispatch.lock",
                expired["request_id"].as_str().unwrap()
            )))
            .unwrap();
        while fs2::FileExt::try_lock_exclusive(&dispatch_lock).is_err() {
            assert!(
                // test-hygiene: allow-wall-clock-deadline Acquiring the forced bookkeeping barrier keeps the original 30s startup bound.
                started.elapsed() < Duration::from_secs(30),
                "sender held the dispatch gate while upstream admission was blocked"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        // test-hygiene: allow-wall-clock-deadline Observe the real fixed 115s work/120s exit budgets; 130s only bounds a hung native test.
        let hangguard = started + Duration::from_secs(130);
        let original_pids = BTreeSet::from([supervisor.pid, sender.pid]);
        loop {
            let stored: serde_json::Value =
                serde_json::from_slice(&fs::read(expired_path).unwrap()).unwrap();
            let pids = native_feedback_worker_pids(repo.path());
            assert_no_native_feedback_worker_windows(&pids);
            assert!(
                pids.is_subset(&original_pids),
                "deadline owner spawned another worker"
            );
            assert_eq!(
                stored["owned"], prior["owned"],
                "blocked bookkeeping changed the attempt"
            );
            assert_eq!(native_owned_attempt(&stored)["phase"], "pre_dispatch");
            assert!(
                stored["issue_number"].is_null(),
                "locked sender reached upstream"
            );
            assert_ne!(stored["state"], "sent");
            if supervisor.exited() && sender.exited() {
                assert!(
                    // test-hygiene: allow-wall-clock-deadline A blocked final save must keep the real supervisor responsible through its fixed exit deadline.
                    started.elapsed() >= Duration::from_secs(120)
                );
                break;
            }
            assert!(
                // test-hygiene: allow-wall-clock-deadline Failure to settle within the extra 10s is a hung real owner, never cleanup success.
                Instant::now() < hangguard,
                "native deadline/cleanup did not settle: {stored}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        drop(dispatch_lock);
        reconcile_unknown(&mut command, &expired, &prior, repo.path(), secret);
        drop(upstream_lock);
        command.env_remove("GH_TOKEN");
    }
    // Only a known Auto job with unavailable policy must remain noninteractive.
    // Fresh unknown-mode intake retains the existing Manual origin policy.
    let known_auto_store = gwt_core::feedback::FeedbackStore::new(
        gwt_core::paths::gwt_project_dir_for_repo_path(repo.path()).join("feedback"),
    );
    let (known_auto, fresh) = known_auto_store
        .submit_auto(
            gwt_core::feedback::auto_report::SafeAutoReport::current(
                gwt_core::feedback::auto_report::KnownOperation::HookHealth,
            ),
            Utc::now(),
        )
        .unwrap();
    assert!(fresh);
    assert!(known_auto.owned.is_none());
    fs::write(
        gwt::issue_monitor::issue_monitor_prefs_path_for_repo_path(repo.path()),
        "{unreadable native prefs",
    )
    .unwrap();
    let unavailable_input = serde_json::json!({"schema_version":1,"operation":"feedback.process","params":{
        "id":known_auto.snapshot.id(),
    }});
    let (unavailable, _) = intake(&mut command, &unavailable_input, secret, "withheld");
    assert_eq!(unavailable["error"], "policy_unavailable");
    let mut malformed = command.spawn().unwrap();
    malformed
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{{\"operation\":\"feedback.create\",\"{secret}\"").as_bytes())
        .unwrap();
    let malformed = malformed.wait_with_output().unwrap();
    assert!(!malformed.status.success());
    assert!(!String::from_utf8_lossy(&malformed.stderr).contains(secret));
    assert!(
        !marker.exists(),
        "Auto attempted a UI-capable gh/auth process"
    );
    fn inspect(dir: &Path, secret: &[u8]) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                inspect(&path, secret);
            } else {
                assert!(
                    !fs::read(path)
                        .unwrap()
                        .windows(secret.len())
                        .any(|bytes| bytes == secret),
                    "raw input reached a durable sink"
                );
            }
        }
    }
    inspect(&home.path().join(".gwt"), secret.as_bytes());
}

fn native_feedback_worker_pids(repo: &Path) -> BTreeSet<u32> {
    let executable = fs::canonicalize(env!("CARGO_BIN_EXE_gwtd")).unwrap();
    let repo = fs::canonicalize(repo).unwrap();
    // Linux task/thread IDs share executable and cwd with the process but
    // cannot lead its process group. Observe actual processes only.
    let system = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::nothing().with_processes(
            sysinfo::ProcessRefreshKind::nothing()
                .with_exe(sysinfo::UpdateKind::Always)
                .with_cwd(sysinfo::UpdateKind::Always)
                .without_tasks(),
        ),
    )
    .expect("initialize test process observation");
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            (process
                .exe()
                .is_some_and(|path| fs::canonicalize(path).is_ok_and(|path| path == executable))
                && process
                    .cwd()
                    .is_some_and(|path| fs::canonicalize(path).is_ok_and(|path| path == repo)))
            .then_some(pid.as_u32())
        })
        .collect()
}

#[cfg(windows)]
fn native_feedback_role_counts(pids: &BTreeSet<u32>) -> (usize, usize) {
    let (supervisors, senders) = native_feedback_role_pids(pids);
    (supervisors.len(), senders.len())
}

#[cfg(windows)]
fn native_feedback_role_pids(pids: &BTreeSet<u32>) -> (BTreeSet<u32>, BTreeSet<u32>) {
    let selected: Vec<_> = pids
        .iter()
        .map(|pid| sysinfo::Pid::from_u32(*pid))
        .collect();
    let mut system = sysinfo::System::new().expect("initialize test process observation");
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&selected),
        true,
        sysinfo::ProcessRefreshKind::nothing()
            .with_cmd(sysinfo::UpdateKind::Always)
            .without_tasks(),
    );
    let mut roles = (BTreeSet::new(), BTreeSet::new());
    for pid in pids {
        let Some(process) = system.process(sysinfo::Pid::from_u32(*pid)) else {
            continue;
        };
        if process
            .cmd()
            .iter()
            .any(|arg| arg == "--gwt-feedback-supervisor")
        {
            roles.0.insert(*pid);
        }
        if process
            .cmd()
            .iter()
            .any(|arg| arg == "--gwt-feedback-sender")
        {
            roles.1.insert(*pid);
        }
    }
    roles
}

#[cfg(windows)]
struct NativeFeedbackProcess {
    handle: windows::Win32::Foundation::HANDLE,
    pid: u32,
    authorized: bool,
}

#[cfg(windows)]
impl NativeFeedbackProcess {
    fn capture(pid: u32, repo: &Path, role: &str) -> Self {
        use windows::Win32::System::Threading::{
            GetProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            PROCESS_TERMINATE,
        };
        assert_ne!(pid, std::process::id());
        // Open first, then verify the candidate while this exact object stays
        // live. A process list or bare PID never authorizes termination.
        let mut owned = Self {
            handle: unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                    false,
                    pid,
                )
            }
            .expect("open actual feedback role"),
            pid,
            authorized: false,
        };
        assert_eq!(unsafe { GetProcessId(owned.handle) }, pid);
        assert!(!owned.exited());
        let mut system = sysinfo::System::new().expect("initialize test process observation");
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_cmd(sysinfo::UpdateKind::Always)
                .with_exe(sysinfo::UpdateKind::Always)
                .with_cwd(sysinfo::UpdateKind::Always)
                .without_tasks(),
        );
        let process = system
            .process(sysinfo::Pid::from_u32(pid))
            .expect("live role");
        assert_eq!(
            fs::canonicalize(process.exe().expect("role executable")).unwrap(),
            fs::canonicalize(env!("CARGO_BIN_EXE_gwtd")).unwrap()
        );
        assert_eq!(
            fs::canonicalize(process.cwd().expect("isolated role checkout")).unwrap(),
            fs::canonicalize(repo).unwrap()
        );
        assert!(process.cmd().iter().any(|arg| arg == role));
        assert!(!owned.exited(), "candidate exited before role verification");
        owned.authorized = true;
        owned
    }

    fn exited(&self) -> bool {
        use windows::Win32::{
            Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::WaitForSingleObject,
        };
        let result = unsafe { WaitForSingleObject(self.handle, 0) };
        assert!(result == WAIT_OBJECT_0 || result == WAIT_TIMEOUT);
        result == WAIT_OBJECT_0
    }

    fn terminate(&self) {
        assert!(self.authorized);
        if !self.exited() {
            unsafe { windows::Win32::System::Threading::TerminateProcess(self.handle, 91) }
                .expect("terminate only the fixture's proven private role");
        }
    }
}

#[cfg(windows)]
impl Drop for NativeFeedbackProcess {
    fn drop(&mut self) {
        use windows::Win32::{
            Foundation::{CloseHandle, WAIT_TIMEOUT},
            System::Threading::{TerminateProcess, WaitForSingleObject},
        };
        // On assertion failure, reclaim only a role whose executable, fixed
        // argument and isolated checkout were verified while its handle lived.
        if self.authorized && unsafe { WaitForSingleObject(self.handle, 0) } == WAIT_TIMEOUT {
            let _ = unsafe { TerminateProcess(self.handle, 91) };
            let _ = unsafe { WaitForSingleObject(self.handle, 5000) };
        }
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(windows)]
fn native_owned_worker_handles(
    receipt: &Path,
    repo: &Path,
) -> (
    serde_json::Value,
    NativeFeedbackProcess,
    NativeFeedbackProcess,
) {
    use std::time::{Duration, Instant};
    // test-hygiene: allow-wall-clock-deadline Keep the original 30s real-worker startup guard for each forced native branch.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(receipt).unwrap()).unwrap();
        let pids = native_feedback_worker_pids(repo);
        assert_no_native_feedback_worker_windows(&pids);
        let (supervisors, senders) = native_feedback_role_pids(&pids);
        assert!(supervisors.len() <= 1 && senders.len() <= 1);
        if stored["state"] == "sending" && supervisors.len() == 1 && senders.len() == 1 {
            assert_eq!(pids.len(), 2);
            assert_eq!(native_owned_attempt(&stored)["phase"], "pre_dispatch");
            let supervisor = NativeFeedbackProcess::capture(
                *supervisors.first().unwrap(),
                repo,
                "--gwt-feedback-supervisor",
            );
            let sender = NativeFeedbackProcess::capture(
                *senders.first().unwrap(),
                repo,
                "--gwt-feedback-sender",
            );
            return (stored, supervisor, sender);
        }
        assert!(
            // test-hygiene: allow-wall-clock-deadline Bound startup while upstream admission keeps both real private roles alive.
            Instant::now() < deadline,
            "owned worker roles did not start: {stored}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(windows)]
fn native_reject_feedback_retry(command: &mut std::process::Command, id: &str, secret: &str) {
    use std::time::{Duration, Instant};
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::json!({"schema_version":1,"operation":"feedback.retry","params":{"id":id}})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    // test-hygiene: allow-wall-clock-deadline Bound the actual retry-rejection CLI without permitting a worker or external call.
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        // test-hygiene: allow-wall-clock-deadline A stuck rejection is a native test failure, never proof of a safe retry refusal.
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("feedback.retry did not reject an uncertain owned attempt");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
}

#[cfg(windows)]
fn native_owned_attempt(stored: &serde_json::Value) -> &serde_json::Value {
    assert_eq!(stored["owned"]["version"], "v1");
    let attempt = &stored["owned"]["attempt"];
    assert!(
        attempt.is_object(),
        "native owner did not register its attempt"
    );
    assert_eq!(attempt["origin"], "inherited_cli_environment");
    attempt
}

#[cfg(windows)]
fn native_owned_cleanup_confirmed(stored: &serde_json::Value) -> bool {
    let attempt = native_owned_attempt(stored);
    assert_eq!(attempt["phase"], "cancel_accepted");
    // These facts are supplied by the exact Child/Job owner. Process or lock
    // disappearance is deliberately not used as an exit/cleanup assertion.
    attempt["exit"] == "confirmed" && attempt["cleanup"] == "confirmed"
}

#[cfg(windows)]
fn assert_no_native_feedback_worker_windows(pids: &BTreeSet<u32>) {
    use std::ffi::c_void;
    type Hwnd = *mut c_void;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(callback: unsafe extern "system" fn(Hwnd, isize) -> i32, data: isize)
            -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
    }
    unsafe extern "system" fn observe(hwnd: Hwnd, data: isize) -> i32 {
        let (pids, seen) = unsafe { &mut *(data as *mut (&BTreeSet<u32>, bool)) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pids.contains(&pid) {
            *seen = true;
        }
        1
    }
    let mut observation = (pids, false);
    assert_ne!(
        unsafe { EnumWindows(observe, &mut observation as *mut _ as isize) },
        0,
        "native window enumeration failed"
    );
    assert!(
        !observation.1,
        "native window appeared in an actual feedback worker"
    );
}

#[test]
fn pr_help_exposes_ready_without_draft_creation_or_transition() {
    let output = isolated_gwtd_command()
        .args(["--help", "pr"])
        .output()
        .expect("pr help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("UTF-8 help");
    assert!(
        help.contains("pr.create") && help.contains("pr.ready"),
        "{help}"
    );
    assert!(!help.contains("pr.draft"), "{help}");
    assert!(!help.contains("add_labels, draft"), "{help}");
}

fn prepared_hook_session() -> (TempDir, TempDir, String) {
    let home = tempfile::tempdir().expect("home tempdir");
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    let session = Session::new(worktree.path(), "work/hook-transport", AgentId::Codex);
    let session_id = session.id.clone();
    session
        .save(&home.path().join(".gwt").join("sessions"))
        .expect("save hook session");
    (home, worktree, session_id)
}

#[test]
fn gwtd_dispatches_internal_hook_cli_without_gui_output() {
    let output = isolated_gwtd_command()
        .args(["__internal", "daemon-hook", "forward"])
        .stdin(Stdio::null())
        .output()
        .expect("run gwtd");

    assert!(
        output.status.success(),
        "gwtd internal hook should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "headless internal hook should not print GUI guidance, got stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn gwtd_help_describes_the_headless_cli_surface() {
    let output = isolated_gwtd_command()
        .arg("--help")
        .output()
        .expect("run gwtd --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("gwtd"));
    assert!(stdout.contains("issue"));
    assert!(stdout.contains("pr"));
    assert!(stdout.contains("hook"));
    assert!(stdout.contains("memory"));
    assert!(
        !stdout.contains("Launch `gwt` instead"),
        "gwtd help must not redirect agent-facing CLI users to the GUI front door"
    );
}

#[test]
fn gwtd_no_args_dispatches_stdin_json_envelope() {
    let home = tempfile::tempdir().expect("home tempdir");
    let project = tempfile::tempdir().expect("project tempdir");
    let project_root = project
        .path()
        .canonicalize()
        .expect("canonical project root");
    let branch = "work/bin-json";
    let session_id = "session-bin-json";
    let work_id = "work-bin-json";
    let run_git = |args: &[&str]| {
        let output = hidden_command("git")
            .args(args)
            .current_dir(&project_root)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_git(&["init", "-q", "-b", branch]);
    run_git(&[
        "remote",
        "add",
        "origin",
        "https://example.invalid/acme/gwtd-bin-json.git",
    ]);

    let mut session = Session::new(&project_root, branch, AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(project_root.clone());
    session.linked_issue_number = Some(3412);
    assert!(
        session.repo_hash.is_some(),
        "fixture origin must set repo hash"
    );
    session
        .save(&home.path().join(".gwt/sessions"))
        .expect("save Session ledger fixture");

    let state_dir = home
        .path()
        .join(".gwt/projects")
        .join(project_scope_hash(&project_root).as_str())
        .join("project-state");
    let current_path = state_dir.join("current.json");
    let works_path = state_dir.join("works.json");
    let tracked_events_path = project_root.join(".gwt/work/events.jsonl");
    let now = Utc::now();
    let mut projection = WorkspaceProjection::default_for_project(&project_root);
    projection.agents.push(WorkspaceAgentSummary {
        session_id: session_id.to_string(),
        window_id: None,
        agent_id: "codex".to_string(),
        display_name: "Codex".to_string(),
        status_category: WorkspaceStatusCategory::Active,
        current_focus: Some("fixture focus".to_string()),
        title_summary: Some("Fixture Work".to_string()),
        worktree_path: Some(project_root.clone()),
        branch: Some(branch.to_string()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: Some(work_id.to_string()),
        updated_at: now,
    });
    save_workspace_projection_to_path(&current_path, &projection)
        .expect("save canonical Session assignment");

    let mut event = WorkEvent::new(WorkEventKind::Start, work_id, now);
    event.title = Some("Fixture Work".to_string());
    event.status_category = Some(WorkspaceStatusCategory::Active);
    event.owner = Some("Issue #3412".to_string());
    event.agent_session_id = Some(session_id.to_string());
    event.agent_id = Some("codex".to_string());
    event.display_name = Some("Codex".to_string());
    event.execution_container = Some(WorkspaceExecutionContainerRef {
        branch: Some(branch.to_string()),
        worktree_path: Some(project_root.clone()),
        pr_number: None,
        pr_url: None,
        pr_state: None,
    });
    let mut work_items = WorkItemsProjection::empty(now);
    let _ = work_items.apply_event(event.clone());
    save_workspace_work_items_projection_to_path(&works_path, &work_items)
        .expect("save assigned WorkItems fixture");
    append_workspace_work_event_to_path(&tracked_events_path, &event)
        .expect("save tracked Work event fixture");

    let mut child = isolated_gwtd_command()
        .current_dir(&project_root)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("GWT_SESSION_ID", session_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(
            br#"{
                "schema_version": 1,
                "operation": "workspace.update",
                "params": {
                    "agent_session": "session-bin-json",
                    "purpose": "Binary JSON envelope",
                    "current_focus": "integration test"
                }
            }"#,
        )
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait gwtd");

    assert!(
        output.status.success(),
        "gwtd JSON envelope should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse gwtd JSON response");
    assert_eq!(
        response.get("ok").and_then(|value| value.as_bool()),
        Some(true),
        "stdout should be success JSON with ok=true, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let projection = load_workspace_projection_from_path(&current_path)
        .expect("load workspace projection")
        .expect("workspace projection should be written under isolated home");
    let agent = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == "session-bin-json")
        .expect("agent upserted by gwtd JSON envelope");
    assert_eq!(agent.title_summary.as_deref(), Some("Binary JSON envelope"));
}

#[test]
fn gwtd_rejects_legacy_family_argv_invocations() {
    for args in [
        ["board", "show"].as_slice(),
        ["issue", "view", "1"].as_slice(),
        ["hook", "register-codex-managed-hook-trust"].as_slice(),
        ["hook", "register-codex-managed-project-trust"].as_slice(),
        ["index", "--help"].as_slice(),
        ["workspace", "update", "--title-summary", "legacy"].as_slice(),
    ] {
        let output = isolated_gwtd_command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("run gwtd legacy argv");

        assert_eq!(
            output.status.code(),
            Some(2),
            "legacy argv must exit 2 for args {args:?}; stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("stdin JSON envelope"),
            "stderr must point agents to JSON envelope for args {args:?}, got: {stderr}"
        );
    }
}

#[test]
fn gwtd_index_help_lists_every_rebuild_scope() {
    let output = isolated_gwtd_command()
        .args(["--help", "index"])
        .output()
        .expect("run gwtd --help index");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("all|issues|specs|memory|discussions|board|files|files-docs"),
        "index help must list every accepted rebuild scope, got: {stdout}"
    );
}

fn linked_codex_hook_project() -> (TempDir, PathBuf) {
    let project = tempfile::tempdir().expect("project tempdir");
    let repo = project.path().join("repo");
    let gitdir = repo.join("project.git/worktrees/linked");
    let worktree = repo.join("work/linked");
    fs::create_dir_all(&gitdir).unwrap();
    fs::create_dir_all(&worktree).unwrap();
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", gitdir.display()),
    )
    .unwrap();
    (project, worktree)
}

fn register_codex_hook_trust(
    worktree: &Path,
    config: &Path,
    mode: Option<&str>,
) -> std::process::Output {
    let mut child = isolated_gwtd_command()
        .env("GWT_HOOK_BIN", env!("CARGO_BIN_EXE_gwtd"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd hook register");
    write!(child.stdin.take().expect("stdin"), "{}", serde_json::json!({
        "schema_version": 1,
        "operation": "hook.register_codex_managed_hook_trust",
        "params": {"project_root": worktree, "codex_config": config, "codex_hook_discovery": mode}
    })).expect("write JSON envelope");
    child.wait_with_output().expect("wait gwtd hook register")
}

#[test]
fn gwtd_hook_register_codex_managed_hook_trust_writes_requested_config() {
    let (_project, worktree) = linked_codex_hook_project();
    let codex_home = tempfile::tempdir().expect("codex tempdir");
    let config_path = codex_home.path().join("config.toml");
    {
        let _pin = gwt_skills::settings_local::ScopedHookBin::set(env!("CARGO_BIN_EXE_gwtd"));
        gwt_skills::generate_codex_hooks_for_mode(
            &worktree,
            gwt_skills::CodexHookDiscoveryMode::Both,
        )
        .expect("generate hooks");
    }

    let output = register_codex_hook_trust(&worktree, &config_path, None);

    assert!(
        output.status.success(),
        "registration should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("hook registration stdout must be JSON");
    assert_eq!(response["ok"].as_bool(), Some(true));
    assert!(
        response["output"]
            .as_str()
            .is_some_and(|output| output.contains("trusted 10")),
        "JSON output field should report trusted hook count, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let config = fs::read_to_string(&config_path).expect("read config");
    assert!(
        config.contains("trusted_hash"),
        "Codex config must receive trusted hashes, got: {config}"
    );
    assert_eq!(
        config.matches("enabled = true").count(),
        10,
        "Codex config must enable every trusted managed hook, got: {config}"
    );
    let parsed: toml::Value = toml::from_str(&config).unwrap();
    let state = parsed["hooks"]["state"].as_table().unwrap();
    let entries = gwt_skills::collect_codex_managed_hook_trust_entries_for_mode(
        &worktree,
        gwt_skills::CodexHookDiscoveryMode::Both,
    )
    .unwrap();
    assert_eq!(state.len(), entries.len());
    for entry in entries {
        assert_eq!(
            state[&entry.key]["trusted_hash"].as_str(),
            Some(entry.trusted_hash.as_str())
        );
    }
    // An explicit narrow scope still registers only the requested location.
    let scoped_config = codex_home.path().join("workspace-home.toml");
    let output = register_codex_hook_trust(&worktree, &scoped_config, Some("workspace-home"));
    assert!(output.status.success(), "{output:?}");
    let scoped: toml::Value = toml::from_str(&fs::read_to_string(scoped_config).unwrap()).unwrap();
    let scoped_state = scoped["hooks"]["state"].as_table().unwrap();
    let expected = gwt_skills::collect_codex_managed_hook_trust_entries_for_mode(
        &worktree,
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
    )
    .unwrap();
    assert_eq!(scoped_state.len(), 5);
    for entry in expected {
        assert_eq!(
            scoped_state[&entry.key]["trusted_hash"].as_str(),
            Some(entry.trusted_hash.as_str())
        );
    }
}

/// Issue #3967 AC-4: this is the front door an operator runs to check a real
/// machine, and it used to answer `trusted 0 gwt-managed Codex hooks` with exit
/// 0 while Codex was still going to stop every launch on `Hooks need review`.
/// Hooks gwt cannot vouch for have to be reported, and the operation has to
/// fail.
#[test]
fn gwtd_hook_register_codex_managed_hook_trust_fails_on_hooks_it_cannot_vouch_for() {
    let (project, worktree) = linked_codex_hook_project();
    let codex_home = tempfile::tempdir().expect("codex tempdir");
    let config_path = codex_home.path().join("config.toml");
    {
        let _pin = gwt_skills::settings_local::ScopedHookBin::set(env!("CARGO_BIN_EXE_gwtd"));
        gwt_skills::generate_codex_hooks_for_mode(
            &worktree,
            gwt_skills::CodexHookDiscoveryMode::WorkspaceHome,
        )
        .expect("generate root hooks");
    }
    // Only the local copy uses a different binary; default registration must
    // not report success based on the valid workspace-home copy.
    let generated_with = project.path().join("Programs").join("GWT").join("gwtd");
    // #4057: pin the generator per thread. `GWT_HOOK_BIN` is process-global, so
    // setting it here would leak this pin into any materialization another test
    // runs at the same time.
    {
        let _pin = gwt_skills::settings_local::ScopedHookBin::set(&generated_with);
        let old_hooks = project.path().join("old-hooks");
        gwt_skills::generate_codex_hooks(&old_hooks).expect("generate old hooks");
        fs::create_dir_all(worktree.join(".codex")).unwrap();
        fs::copy(
            old_hooks.join(".codex/hooks.json"),
            worktree.join(".codex/hooks.json"),
        )
        .expect("leave an older local copy beside current root hooks");
    }

    let output = register_codex_hook_trust(&worktree, &config_path, None);

    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("hook registration stdout must be JSON");
    assert_eq!(
        response["ok"].as_bool(),
        Some(false),
        "registration must not report success while hooks stay untrusted, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        response["output"]
            .as_str()
            .is_some_and(|output| output.contains("Hooks need review")),
        "the refusal must name the launch Codex would stop, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let reason = response["output"].as_str().unwrap();
    let local = gwt_skills::codex_hook_trust_key_path(&worktree.join(".codex/hooks.json")).unwrap();
    assert!(reason.contains(&local.display().to_string()), "{reason}");
    assert!(
        reason.contains("expected_command=") && reason.contains("actual_command="),
        "{reason}"
    );
    assert!(
        reason.contains("hook.doctor") && reason.contains("hook.register_codex_managed_hook_trust"),
        "{reason}"
    );
}

#[test]
fn gwtd_hook_register_codex_managed_project_trust_writes_exact_project() {
    let project = tempfile::tempdir().expect("project tempdir");
    let codex_home = tempfile::tempdir().expect("codex tempdir");
    let config_path = codex_home.path().join("config.toml");
    let canonical_project = gwt_core::paths::normalize_windows_child_process_path(
        &fs::canonicalize(project.path()).expect("canonical project"),
    );

    let mut child = isolated_gwtd_command()
        .env("CODEX_HOME", codex_home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd project trust register");
    write!(
        child.stdin.take().expect("stdin"),
        "{}",
        serde_json::json!({
            "schema_version": 1,
            "operation": "hook.register_codex_managed_project_trust",
            "params": {
                "project_root": project.path().to_str().expect("project path utf8"),
                "codex_config": config_path.to_str().expect("config path utf8"),
            }
        })
    )
    .expect("write JSON envelope");
    let output = child
        .wait_with_output()
        .expect("wait gwtd project trust register");

    assert!(
        output.status.success(),
        "registration should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("project trust registration stdout must be JSON");
    assert_eq!(response["ok"].as_bool(), Some(true));
    assert!(
        response["output"]
            .as_str()
            .is_some_and(|output| output.contains("trusted gwt-managed Codex worktree")),
        "JSON output should report the exact project trust registration, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(config_path).expect("read config")).unwrap();
    assert_eq!(
        config["projects"][canonical_project.to_string_lossy().as_ref()]["trust_level"].as_str(),
        Some("trusted")
    );
}

#[test]
fn gwtd_hook_register_codex_managed_project_trust_refuses_explicit_untrusted() {
    let project = tempfile::tempdir().expect("project tempdir");
    let codex_home = tempfile::tempdir().expect("codex tempdir");
    let config_path = codex_home.path().join("config.toml");
    let canonical_project = gwt_core::paths::normalize_windows_child_process_path(
        &fs::canonicalize(project.path()).expect("canonical project"),
    );
    let mut project_config = toml::Table::new();
    project_config.insert(
        "trust_level".to_string(),
        toml::Value::String("untrusted".to_string()),
    );
    let mut projects = toml::Table::new();
    projects.insert(
        canonical_project.to_string_lossy().into_owned(),
        toml::Value::Table(project_config),
    );
    let mut root = toml::Table::new();
    root.insert("projects".to_string(), toml::Value::Table(projects));
    let config = toml::to_string_pretty(&toml::Value::Table(root)).unwrap();
    fs::write(&config_path, &config).expect("seed explicit untrusted config");

    let mut child = isolated_gwtd_command()
        .env("CODEX_HOME", codex_home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd project trust register");
    write!(
        child.stdin.take().expect("stdin"),
        "{}",
        serde_json::json!({
            "schema_version": 1,
            "operation": "hook.register_codex_managed_project_trust",
            "params": {
                "project_root": project.path().to_str().expect("project path utf8"),
                "codex_config": config_path.to_str().expect("config path utf8"),
            }
        })
    )
    .expect("write JSON envelope");
    let output = child
        .wait_with_output()
        .expect("wait gwtd project trust register");

    assert!(!output.status.success());
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("project trust refusal stdout must be JSON");
    assert_eq!(response["ok"].as_bool(), Some(false));
    assert!(
        response["error"]
            .as_str()
            .is_some_and(|error| error.contains("explicitly untrusted")),
        "JSON error should preserve the fail-closed reason, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(fs::read_to_string(config_path).unwrap(), config);
}

#[test]
fn gwtd_hook_register_codex_managed_project_trust_uses_effective_codex_home() {
    let project = tempfile::tempdir().expect("project tempdir");
    let codex_home = tempfile::tempdir().expect("codex tempdir");
    let config_path = codex_home.path().join("config.toml");
    let canonical_project = gwt_core::paths::normalize_windows_child_process_path(
        &fs::canonicalize(project.path()).expect("canonical project"),
    );

    let mut child = isolated_gwtd_command()
        .env("CODEX_HOME", codex_home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd project trust register");
    write!(
        child.stdin.take().expect("stdin"),
        "{}",
        serde_json::json!({
            "schema_version": 1,
            "operation": "hook.register_codex_managed_project_trust",
            "params": {
                "project_root": project.path().to_str().expect("project path utf8"),
            }
        })
    )
    .expect("write JSON envelope");
    let output = child
        .wait_with_output()
        .expect("wait gwtd project trust register");

    assert!(
        output.status.success(),
        "registration should use CODEX_HOME, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(config_path).expect("read config")).unwrap();
    assert_eq!(
        config["projects"][canonical_project.to_string_lossy().as_ref()]["trust_level"].as_str(),
        Some("trusted")
    );
}

#[test]
fn gwtd_hook_register_codex_managed_project_trust_refuses_arbitrary_host_config() {
    let project = tempfile::tempdir().expect("project tempdir");
    let stable_codex_home = tempfile::tempdir().expect("stable Codex tempdir");
    let arbitrary_codex_home = tempfile::tempdir().expect("arbitrary Codex tempdir");
    let arbitrary_config = arbitrary_codex_home.path().join("config.toml");

    let mut child = isolated_gwtd_command()
        .env("CODEX_HOME", stable_codex_home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd project trust register");
    write!(
        child.stdin.take().expect("stdin"),
        "{}",
        serde_json::json!({
            "schema_version": 1,
            "operation": "hook.register_codex_managed_project_trust",
            "params": {
                "project_root": project.path().to_str().expect("project path utf8"),
                "codex_config": arbitrary_config.to_str().expect("config path utf8"),
            }
        })
    )
    .expect("write JSON envelope");
    let output = child
        .wait_with_output()
        .expect("wait gwtd project trust register");

    assert!(!output.status.success());
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("project trust refusal stdout must be JSON");
    assert_eq!(response["ok"].as_bool(), Some(false));
    assert!(
        response["error"]
            .as_str()
            .is_some_and(|error| error.contains("process-stable/default")),
        "refusal must identify the supported Host config boundary: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !arbitrary_config.exists(),
        "the explicit arbitrary Host config must remain untouched"
    );
}

#[test]
fn gwtd_internal_docker_marker_refuses_an_explicit_codex_config() {
    let project = tempfile::tempdir().expect("project tempdir");
    let stable_codex_home = tempfile::tempdir().expect("stable Codex tempdir");
    let arbitrary_codex_home = tempfile::tempdir().expect("arbitrary Codex tempdir");
    let arbitrary_config = arbitrary_codex_home.path().join("config.toml");

    let output = isolated_gwtd_command()
        .args([
            "__internal",
            "daemon-hook",
            "register-codex-managed-project-trust",
            "--project-root",
            project.path().to_str().expect("project path utf8"),
            "--runtime-target",
            "docker",
            "--codex-config",
            arbitrary_config.to_str().expect("config path utf8"),
        ])
        .env("CODEX_HOME", stable_codex_home.path())
        .stdin(Stdio::null())
        .output()
        .expect("run internal Docker project trust registration");

    assert!(!output.status.success());
    assert!(
        !arbitrary_config.exists(),
        "a caller-controlled Docker marker must not authorize an explicit config"
    );
}

#[test]
fn gwtd_managed_hook_event_remains_argv_transport_exception() {
    let (home, worktree, session_id) = prepared_hook_session();
    let output = isolated_gwtd_command()
        .current_dir(worktree.path())
        .args(["hook", "event", "SessionStart"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("GWT_SESSION_ID", &session_id)
        .env_remove("GWT_SESSION_RUNTIME_PATH")
        .stdin(Stdio::null())
        .output()
        .expect("run gwtd hook event");

    assert!(
        output.status.success(),
        "managed hook transport should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("hookSpecificOutput"),
        "SessionStart should keep the managed hook stdout contract, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn gwtd_provider_hook_event_remains_argv_transport_exception() {
    let (home, worktree, session_id) = prepared_hook_session();
    let mut child = isolated_gwtd_command()
        .current_dir(worktree.path())
        .args(["hook", "provider-event", "opencode", "session.created"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("GWT_SESSION_ID", &session_id)
        .env_remove("GWT_SESSION_RUNTIME_PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd provider hook event");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(br#"{"sessionId":"provider-session"}"#)
        .expect("write provider event payload");
    let output = child.wait_with_output().expect("wait provider hook event");

    assert!(
        output.status.success(),
        "provider hook transport should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("hookSpecificOutput"),
        "provider SessionStart should keep the hook stdout contract, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Initialize a worktree whose `origin` is `akiojin/gwt`, the repository whose
/// generated hook assets this suite inspects.
fn init_gwt_origin_repo(worktree: &Path) {
    assert!(hidden_command("git")
        .arg("init")
        .arg("-q")
        .arg(worktree)
        .status()
        .expect("git init")
        .success());
    assert!(hidden_command("git")
        .arg("-C")
        .arg(worktree)
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/akiojin/gwt.git"
        ])
        .status()
        .expect("git remote add")
        .success());
}

/// Concatenate every generated text artifact under `dir` (skipping `.git`) so
/// the gwtd `hook <subcommand>` invocations from Claude/Codex settings and the
/// OpenCode/OpenClaw/Hermes provider bridges land in one corpus.
fn collect_generated_text(dir: &Path, out: &mut String) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some(".git") {
            continue;
        }
        if path.is_dir() {
            collect_generated_text(&path, out);
        } else if let Ok(content) = fs::read_to_string(&path) {
            out.push_str(&content);
            out.push('\n');
        }
    }
}

/// Extract the gwtd `hook <subcommand>` keywords that generated artifacts
/// invoke. Shell (`"$gwt_bin" hook event Stop`) and JS-array
/// (`["hook", "provider-event", ...]`) call forms both reduce to the same token
/// stream once quoting/bracket punctuation is flattened to whitespace, so a
/// `hook` token whose predecessor references the gwt hook binary yields the
/// routing subcommand. Anchoring on the binary predecessor keeps prose like the
/// OpenClaw manifest's "plugin hook events" out of the result set.
fn generated_hook_subcommands(corpus: &str) -> BTreeSet<String> {
    let flattened: String = corpus
        .chars()
        .map(|c| match c {
            '"' | '\'' | ',' | '[' | ']' | '(' | ')' | ';' | '{' | '}' | '|' | '=' => ' ',
            other => other,
        })
        .collect();
    let tokens: Vec<&str> = flattened.split_whitespace().collect();
    let references_hook_bin = |token: &str| {
        let normalized = token.to_ascii_lowercase();
        normalized.contains("gwtd")
            || normalized.contains("gwt_bin")
            || normalized.contains("gwtbin")
            || normalized.contains("gwt_hook_bin")
    };
    let is_subcommand = |token: &str| {
        let mut chars = token.chars();
        matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
            && token.chars().all(|c| c.is_ascii_lowercase() || c == '-')
    };

    let mut subcommands = BTreeSet::new();
    for index in 1..tokens.len().saturating_sub(1) {
        if tokens[index] == "hook"
            && references_hook_bin(tokens[index - 1])
            && is_subcommand(tokens[index + 1])
        {
            subcommands.insert(tokens[index + 1].to_string());
        }
    }
    subcommands
}

/// Run `gwtd hook <args>` with an isolated home and report whether the binary
/// rejected the argv with the legacy-transport error that broke issue #3178.
fn gwtd_hook_argv_rejected(args: &[&str], stdin: &str) -> (bool, String) {
    let home = tempfile::tempdir().expect("home tempdir");
    let cwd = tempfile::tempdir().expect("cwd tempdir");
    let mut child = isolated_gwtd_command()
        .current_dir(cwd.path())
        .args(args)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env_remove("GWT_SESSION_RUNTIME_PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd hook argv");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write hook stdin");
    let output = child.wait_with_output().expect("wait gwtd hook argv");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let rejected = stderr.contains("legacy argv invocation is disabled");
    (rejected, stderr)
}

/// Regression guard for issue #3178: every managed-hook command that generation
/// emits must stay inside gwtd's argv transport allowlist. A generated command
/// with no matching `is_allowed_argv_exception` entry hits the legacy-argv
/// rejection on every hook invocation. This test derives the subcommands from
/// the actual generated artifacts (not a hard-coded list) and runs each through
/// the real binary, so a new generation site that drifts ahead of the allowlist
/// fails here instead of silently at runtime.
#[test]
fn generated_managed_hook_commands_stay_within_gwtd_argv_allowlist() {
    let worktree = tempfile::tempdir().expect("worktree tempdir");
    init_gwt_origin_repo(worktree.path());

    gwt_skills::generate_settings_local(worktree.path()).expect("generate claude settings");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("generate codex hooks");
    gwt_skills::generate_opencode_hooks(worktree.path()).expect("generate opencode hooks");
    gwt_skills::generate_openclaw_hooks(worktree.path()).expect("generate openclaw hooks");
    gwt_skills::generate_hermes_hooks(worktree.path()).expect("generate hermes hooks");

    let mut corpus = String::new();
    collect_generated_text(worktree.path(), &mut corpus);
    let subcommands = generated_hook_subcommands(&corpus);

    for expected in ["event", "provider-event"] {
        assert!(
            subcommands.contains(expected),
            "generation must still emit the `hook {expected}` managed-hook command; \
             discovered subcommands: {subcommands:?}"
        );
    }

    for subcommand in &subcommands {
        let (args, stdin): (Vec<&str>, &str) = match subcommand.as_str() {
            "event" => (vec!["hook", "event", "SessionStart"], ""),
            "provider-event" => (
                vec!["hook", "provider-event", "opencode", "session.created"],
                "{\"sessionId\":\"guard\"}",
            ),
            other => panic!(
                "generation emits an unmapped gwtd hook subcommand `{other}`. Add a representative \
                 argv here and confirm gwtd's is_allowed_argv_exception accepts it, or the same \
                 generation↔binary drift that broke issue #3178 will ship undetected."
            ),
        };
        let (rejected, stderr) = gwtd_hook_argv_rejected(&args, stdin);
        assert!(
            !rejected,
            "generated `hook {subcommand}` argv was rejected by gwtd's legacy-argv guard \
             (issue #3178 regression); stderr: {stderr}"
        );
    }
}

/// SPEC #3164 AC-R2: the retired `improvement.*` operations are gone from the
/// envelope dispatcher, so they must land on the pre-existing unknown-operation
/// rejection rather than on a bespoke refusal code.
#[test]
fn retired_improvement_operations_hit_the_unknown_operation_rejection() {
    let home = tempfile::tempdir().expect("home tempdir");
    let repo = tempfile::tempdir().expect("repo tempdir");

    for operation in ["improvement.list", "improvement.capture"] {
        let mut child = isolated_gwtd_command()
            .current_dir(repo.path())
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn gwtd");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(
                format!("{{\"schema_version\":1,\"operation\":\"{operation}\",\"params\":{{}}}}")
                    .as_bytes(),
            )
            .expect("write envelope");
        let output = child.wait_with_output().expect("wait gwtd");

        assert!(
            !output.status.success(),
            "`{operation}` must be refused after the improvement subsystem retirement"
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("unknown"),
            "`{operation}` must be refused through the existing unknown-operation path, got: {combined}"
        );
    }
}

/// SPEC #3164 AC-R3: `gwtd hook gwt-self-improvement-stop` lost its argv
/// transport exception along with the hook, so the binary must refuse it
/// instead of running a retired gate.
#[test]
fn retired_self_improvement_stop_hook_is_no_longer_an_argv_exception() {
    let home = tempfile::tempdir().expect("home tempdir");
    let repo = tempfile::tempdir().expect("repo tempdir");

    let output = isolated_gwtd_command()
        .current_dir(repo.path())
        .args(["hook", "gwt-self-improvement-stop"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .stdin(Stdio::null())
        .output()
        .expect("run retired self-improvement hook");

    assert!(
        !output.status.success(),
        "the retired self-improvement Stop hook must not be dispatched"
    );
    assert!(
        output.stdout.is_empty(),
        "the retired hook must emit no hook decision, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("gwt-self-improvement-stop"),
        "the retired hook must not be advertised as a transport exception, got: {stderr}"
    );
}

/// Issue #3606: a JSON operation that acts on the project store must name the
/// store it landed in.
///
/// The PM ran `issue.monitor.priority.move` against a `project_root` whose
/// identity did not resolve, got `ok: true` plus a successful readback, and
/// nearly reported the wrong queue state to the user — the write had gone to an
/// isolated path-fallback store no running gwt reads. `ok: true` proves the
/// operation ran; it never proved *where*. These tests pin the proof onto the
/// envelope so the landing is checkable instead of inferable from file mtimes.
const STORE_LANDING_ORIGIN: &str = "https://example.invalid/acme/store-landing.git";

struct StoreLandingFixture {
    home: TempDir,
    _temp: TempDir,
    layout_root: PathBuf,
    worktree: PathBuf,
}

fn store_landing_git(cwd: &Path, args: &[&str]) {
    let output = hidden_command("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git store-landing fixture command");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A Nested Bare + Worktree layout: `<root>/gwt.git` plus `<root>/work/develop`.
/// This is the shape `E:/gwt` has in the report, where the layout root itself is
/// not a repository but still scopes exactly one project.
fn store_landing_fixture() -> StoreLandingFixture {
    let home = tempfile::tempdir().expect("home tempdir");
    let temp = tempfile::tempdir().expect("layout tempdir");
    let layout_root = temp.path().join("workbench");
    let bare = layout_root.join("gwt.git");
    let bootstrap = layout_root.join(".bootstrap");
    let worktree = layout_root.join("work").join("develop");
    fs::create_dir_all(worktree.parent().expect("work dir")).expect("create work dir");

    store_landing_git(
        temp.path(),
        &["init", "--bare", bare.to_str().expect("bare path utf8")],
    );
    store_landing_git(&bare, &["remote", "add", "origin", STORE_LANDING_ORIGIN]);
    store_landing_git(
        &layout_root,
        &[
            "clone",
            bare.to_str().expect("bare path utf8"),
            ".bootstrap",
        ],
    );
    store_landing_git(&bootstrap, &["checkout", "-b", "develop"]);
    store_landing_git(
        &bootstrap,
        &[
            "-c",
            "user.name=gwt-test",
            "-c",
            "user.email=gwt-test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    store_landing_git(&bootstrap, &["push", "origin", "develop"]);
    fs::remove_dir_all(&bootstrap).expect("remove bootstrap clone");
    store_landing_git(
        &bare,
        &[
            "worktree",
            "add",
            worktree.to_str().expect("worktree path utf8"),
            "develop",
        ],
    );

    StoreLandingFixture {
        home,
        _temp: temp,
        layout_root,
        worktree,
    }
}

fn run_store_landing_envelope(
    home: &Path,
    cwd: &Path,
    envelope: &serde_json::Value,
) -> serde_json::Value {
    let mut child = isolated_gwtd_command()
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gwtd");
    write!(child.stdin.take().expect("stdin"), "{envelope}").expect("write JSON envelope");
    let output = child.wait_with_output().expect("wait gwtd");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "gwtd stdout must be a JSON envelope ({error}); stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn issue_monitor_status_names_the_project_store_it_resolved() {
    let fixture = store_landing_fixture();
    let expected = compute_repo_hash(STORE_LANDING_ORIGIN);

    let response = run_store_landing_envelope(
        fixture.home.path(),
        &fixture.layout_root,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.status",
            "params": { "project_root": fixture.layout_root.to_str().expect("layout path utf8") },
        }),
    );

    assert_eq!(response["ok"].as_bool(), Some(true), "response: {response}");
    let store = &response["project_store"];
    assert_eq!(
        store["hash"].as_str(),
        Some(expected.as_str()),
        "the envelope must name the store the operation resolved; response: {response}"
    );
    assert_eq!(
        store["identity_resolved"].as_bool(),
        Some(true),
        "a Nested Bare + Worktree layout root resolves a repository identity; response: {response}"
    );
    assert_eq!(
        store["source"].as_str(),
        Some("nested_bare_repository"),
        "response: {response}"
    );
    let store_path = store["store_path"].as_str().expect("store_path");
    assert!(
        Path::new(store_path).ends_with(expected.as_str()),
        "store_path must point at the store directory the caller can inspect, got {store_path}"
    );
}

#[test]
fn issue_monitor_status_reports_the_same_store_from_a_linked_worktree() {
    let fixture = store_landing_fixture();
    let expected = compute_repo_hash(STORE_LANDING_ORIGIN);

    // Issue #3606 AC-5': a linked worktree — the shape of a gwt `work/` checkout
    // and of the PM's `~/.gwt/projects/<hash>/pm/worktree` — must converge on the
    // same store as the layout root it was materialized from.
    let response = run_store_landing_envelope(
        fixture.home.path(),
        &fixture.worktree,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.status",
            "params": { "project_root": fixture.worktree.to_str().expect("worktree path utf8") },
        }),
    );

    assert_eq!(response["ok"].as_bool(), Some(true), "response: {response}");
    assert_eq!(
        response["project_store"]["hash"].as_str(),
        Some(expected.as_str()),
        "a linked worktree must resolve the repository identity store; response: {response}"
    );
    assert_eq!(
        response["project_store"]["source"].as_str(),
        Some("origin"),
        "response: {response}"
    );
}

#[test]
fn issue_monitor_write_reports_an_isolated_path_fallback_landing() {
    let home = tempfile::tempdir().expect("home tempdir");
    let unresolvable = tempfile::tempdir().expect("project tempdir");
    let expected = compute_path_hash(unresolvable.path());

    let response = run_store_landing_envelope(
        home.path(),
        unresolvable.path(),
        &serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.config.set",
            "params": {
                "project_root": unresolvable.path().to_str().expect("project path utf8"),
                "max_active": 7,
            },
        }),
    );

    // The write still succeeds — the store is simply isolated. What must change
    // is that the caller can see that from the response alone.
    assert_eq!(response["ok"].as_bool(), Some(true), "response: {response}");
    let store = &response["project_store"];
    assert_eq!(
        store["identity_resolved"].as_bool(),
        Some(false),
        "a path-fallback landing must be visible without reading store mtimes; response: {response}"
    );
    assert_eq!(
        store["source"].as_str(),
        Some("path_fallback"),
        "response: {response}"
    );
    assert_eq!(
        store["hash"].as_str(),
        Some(expected.as_str()),
        "response: {response}"
    );
}

#[test]
fn issue_monitor_status_lists_the_candidates_of_an_ambiguous_layout_root() {
    let fixture = store_landing_fixture();
    let second = fixture.layout_root.join("aa-other.git");
    store_landing_git(
        &fixture.layout_root,
        &["init", "--bare", second.to_str().expect("second path utf8")],
    );
    store_landing_git(
        &second,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/acme/other.git",
        ],
    );

    let response = run_store_landing_envelope(
        fixture.home.path(),
        &fixture.layout_root,
        &serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.status",
            "params": { "project_root": fixture.layout_root.to_str().expect("layout path utf8") },
        }),
    );

    let store = &response["project_store"];
    assert_eq!(
        store["source"].as_str(),
        Some("ambiguous_nested_bare_repositories"),
        "response: {response}"
    );
    assert_eq!(
        store["identity_resolved"].as_bool(),
        Some(false),
        "response: {response}"
    );
    let origins: BTreeSet<&str> = store["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .filter_map(|candidate| candidate["normalized_origin"].as_str())
        .collect();
    assert_eq!(
        origins,
        BTreeSet::from([
            "example.invalid/acme/other",
            "example.invalid/acme/store-landing"
        ]),
        "an ambiguous layout root must say which origins it refused to choose between; \
         response: {response}"
    );
}
