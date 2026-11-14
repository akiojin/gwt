//! SPEC-3431 Phase 24 / T252: IPC/runtime durability coverage for registered-PM
//! stop, failover, and stalled recovery.
//!
//! The state-layer matrix (T-086/T-087/T-087c) proves the stop machine and the
//! daemon rebase converge on in-memory structures. This file pins the same
//! contract through the real boundaries those tests skipped: the durable prefs
//! file behind its lock, and the daemon control socket.
//!
//! - a daemon that accepts the connection and then hangs must cost a bounded
//!   wait, never a hang, and must report an outcome-unknown error (no silent
//!   success, no local-fallback authority grab);
//! - a stop written to disk must survive the daemon's own commit path
//!   (`rebase_daemon_driver_prefs` inside `mutate_issue_monitor_prefs`) and
//!   must not be resurrected by the daemon's in-memory view;
//! - an unstoppable target (prefs not writable) must fail closed with a typed
//!   error and leave the running launch untouched — a phantom "stopped" that
//!   never persisted is the one outcome the PM can never recover from;
//! - a restart stays teardown/claim-release fenced; after settlement its queue,
//!   receipt, fresh-session requirement, and pinned profile survive process
//!   reload and immediate-scan publication failure.

use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::{
    io::{BufRead, BufReader},
    os::unix::net::UnixListener,
};

#[cfg(unix)]
use fs2::FileExt;
use gwt::{
    scan_issue_monitor_candidates, IssueMonitorConfig, IssueMonitorIssue, IssueMonitorIssueState,
    IssueMonitorLaunchSessionStrategy, IssueMonitorPrefs, IssueMonitorState,
    IssueMonitorStopOutcome, IssueMonitorStopTarget, MonitorInboxState,
};
#[cfg(unix)]
use gwt_core::daemon::{
    persist_endpoint, ClientFrame, DaemonEndpoint, DaemonFrame, DaemonStatus, IpcHandshakeRequest,
    IpcHandshakeResponse, RuntimeScope, RuntimeTarget, DAEMON_PROTOCOL_VERSION,
};
use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
use tempfile::TempDir;

const NOW: &str = "2026-08-09T05:00:00Z";
const PHASE24_IO_TIMEOUT: Duration = Duration::from_secs(5);
const PHASE24_OPERATION_TIMEOUT: Duration = Duration::from_secs(8);
const PHASE24_JOIN_TIMEOUT: Duration = Duration::from_secs(15);
const PHASE24_MIN_CLEANUP_CUSHION: Duration = Duration::from_secs(6);
const PHASE24_POLL_INTERVAL: Duration = Duration::from_millis(20);
const PHASE24_CAPTURE_LIMIT: u64 = 1024 * 1024;

fn phase24_assert_timeout_ordering() {
    // A timed-out gwtd or server gets seven wall-clock seconds after its
    // eight-second operation deadline to drop sockets and kill/reap children.
    // Keep the asserted minimum strictly below that actual cushion so the
    // wrapper completion wait can never expire before inner cleanup starts.
    assert!(
        PHASE24_JOIN_TIMEOUT > PHASE24_OPERATION_TIMEOUT + PHASE24_MIN_CLEANUP_CUSHION,
        "wrapper join must outlive the inner operation deadline and cleanup cushion"
    );
}

struct Phase24ChildGuard(Option<Child>);

impl Drop for Phase24ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = phase24_kill_and_reap_child(&mut child);
        }
    }
}

fn phase24_kill_and_reap_child(
    child: &mut Child,
) -> (
    std::io::Result<()>,
    std::io::Result<Option<std::process::ExitStatus>>,
) {
    let kill = child.kill();
    let deadline = Instant::now() + PHASE24_MIN_CLEANUP_CUSHION;
    let reap = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(Some(status)),
            Ok(None) if Instant::now() < deadline => thread::sleep(PHASE24_POLL_INTERVAL),
            Ok(None) => break Ok(None),
            Err(error) => break Err(error),
        }
    };
    (kill, reap)
}

struct Phase24BoundedThread<T> {
    label: &'static str,
    completion: mpsc::Receiver<()>,
    handle: Option<thread::JoinHandle<T>>,
}

impl<T> Phase24BoundedThread<T> {
    fn join(mut self) -> thread::Result<T> {
        phase24_assert_timeout_ordering();
        if self.completion.recv_timeout(PHASE24_JOIN_TIMEOUT).is_err() {
            let label = self.label;
            self.finish_or_abort_after_timeout();
            return Err(Box::new(format!(
                "{label} did not finish within {:?}",
                PHASE24_JOIN_TIMEOUT
            )));
        }
        self.handle.take().expect("bounded thread handle").join()
    }

    fn finish_or_abort_after_timeout(&mut self) {
        let deadline = Instant::now() + PHASE24_MIN_CLEANUP_CUSHION;
        let handle = self.handle.as_ref().expect("bounded thread handle");
        while !handle.is_finished() && Instant::now() < deadline {
            thread::sleep(PHASE24_POLL_INTERVAL);
        }
        if !handle.is_finished() {
            eprintln!(
                "{} remained live after {:?} plus {:?}; aborting the test process to prevent a detached thread from escaping the global test environment",
                self.label, PHASE24_JOIN_TIMEOUT, PHASE24_MIN_CLEANUP_CUSHION
            );
            std::process::abort();
        }
        let _ = self.handle.take().expect("bounded thread handle").join();
    }
}

impl<T> Drop for Phase24BoundedThread<T> {
    fn drop(&mut self) {
        if self.handle.is_some() {
            self.finish_or_abort_after_timeout();
        }
    }
}

fn phase24_spawn_bounded_thread<T, F>(label: &'static str, run: F) -> Phase24BoundedThread<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (completion_tx, completion_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
        let _ = completion_tx.send(());
        match outcome {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    });
    Phase24BoundedThread {
        label,
        completion: completion_rx,
        handle: Some(handle),
    }
}

fn phase24_read_capture(file: &mut std::fs::File) -> String {
    file.seek(SeekFrom::Start(0))
        .expect("rewind gwtd output capture");
    let length = file.metadata().expect("inspect gwtd output capture").len();
    let mut output = Vec::new();
    file.take(PHASE24_CAPTURE_LIMIT)
        .read_to_end(&mut output)
        .expect("read gwtd output capture");
    if length > PHASE24_CAPTURE_LIMIT {
        output.extend_from_slice(b"\n...[capture truncated at 1 MiB]");
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn issue(number: u64) -> IssueMonitorIssue {
    IssueMonitorIssue {
        number,
        title: format!("Issue {number}"),
        labels: vec!["auto-merge".to_string()],
        state: IssueMonitorIssueState::Open,
        body: None,
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
    }
}

/// A monitor with one launched issue bound to `window-1`, as the daemon and
/// the CLI would both observe it.
fn launched_monitor() -> IssueMonitorState {
    let mut monitor = IssueMonitorState::with_prefs(
        IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..IssueMonitorConfig::default()
        },
        IssueMonitorPrefs {
            enabled: true,
            ..IssueMonitorPrefs::default()
        },
    );
    scan_issue_monitor_candidates(&mut monitor, &[issue(42)], NOW);
    monitor.complete_active_launch(42, "tab-1::window-1");
    monitor
}

fn stop_target() -> IssueMonitorStopTarget {
    IssueMonitorStopTarget {
        issue_number: 42,
        claim_id: None,
        delivery_id: None,
        window_id: Some("tab-1::window-1".to_string()),
    }
}

fn phase24_launch_profile(agent_id: &str, model: &str) -> gwt::IssueMonitorLaunchProfile {
    gwt::IssueMonitorLaunchProfile {
        agent_id: agent_id.to_string(),
        model: Some(model.to_string()),
        reasoning: Some("high".to_string()),
        version: Some("phase-24".to_string()),
        session_mode: gwt_agent::SessionMode::Normal,
        skip_permissions: true,
        codex_fast_mode: false,
        runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
        docker_service: None,
        docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
        windows_shell: None,
    }
}

fn phase24_generated_launch() -> IssueMonitorState {
    let mut monitor = IssueMonitorState::with_prefs(
        IssueMonitorConfig {
            enabled: true,
            max_active: 2,
            ..IssueMonitorConfig::default()
        },
        IssueMonitorPrefs {
            enabled: true,
            launch_profile: Some(phase24_launch_profile("codex", "gpt-source")),
            ..IssueMonitorPrefs::default()
        },
    );
    scan_issue_monitor_candidates(&mut monitor, &[issue(42)], NOW);
    assert!(monitor.apply_confirmed_claim(
        42,
        "claim-42-generation-1".to_string(),
        "host/pm-session-a",
        "effect-42-generation-1",
        NOW,
    ));
    assert!(monitor.claim_launch_delivery(
        42,
        "launch:effect-42-generation-1",
        "app-runtime-a",
        4242,
        "tab-1::agent-42",
        |_| false,
    ));
    assert!(monitor.mark_launch_delivery_materialized(
        42,
        "launch:effect-42-generation-1",
        "app-runtime-a",
        "tab-1::agent-42",
    ));
    assert!(monitor.mark_launch_delivery_workspace_durable(
        42,
        "launch:effect-42-generation-1",
        "app-runtime-a",
        "tab-1::agent-42",
    ));
    assert!(monitor.complete_active_launch_delivery(
        42,
        "tab-1::agent-42",
        Some("launch:effect-42-generation-1"),
    ));
    monitor
}

fn phase24_register_pm(project: &std::path::Path, session_id: &str) {
    let pm_prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(project);
    let isolated_gwt_home = gwt_core::paths::gwt_home();
    assert!(
        pm_prefs_path.starts_with(&isolated_gwt_home),
        "PM registration escaped isolated gwt home: prefs={}, home={}",
        pm_prefs_path.display(),
        isolated_gwt_home.display()
    );
    let registration = serde_json::from_value(serde_json::json!({
        "session_id": session_id,
        "agent_id": "codex",
        "worktree_path": project,
        "created_at": "2026-08-15T00:00:00Z",
    }))
    .expect("PM registration fixture");
    gwt::pm_registry::try_register_pm(&pm_prefs_path, registration, |_| false)
        .expect("register PM");
}

fn phase24_run_gwtd(
    project: &std::path::Path,
    home: &std::path::Path,
    session_id: &str,
    request: serde_json::Value,
) -> (std::process::ExitStatus, serde_json::Value) {
    phase24_assert_timeout_ordering();
    assert_eq!(
        gwt_core::paths::gwt_home(),
        home.join(".gwt"),
        "parent and helper thread must resolve the same isolated gwt home as child HOME"
    );
    let mut stdout_capture = tempfile::tempfile().expect("create gwtd stdout capture");
    let mut stderr_capture = tempfile::tempfile().expect("create gwtd stderr capture");
    let child = gwt_core::process::hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(project)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GWT_PROJECT_ROOT", project)
        .env("GWT_SESSION_ID", session_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(
            stdout_capture
                .try_clone()
                .expect("clone gwtd stdout capture"),
        ))
        .stderr(Stdio::from(
            stderr_capture
                .try_clone()
                .expect("clone gwtd stderr capture"),
        ))
        .spawn()
        .expect("spawn short-lived gwtd");
    let mut child = Phase24ChildGuard(Some(child));
    {
        let mut stdin = child
            .0
            .as_mut()
            .expect("guarded gwtd child")
            .stdin
            .take()
            .expect("gwtd stdin");
        stdin
            .write_all(request.to_string().as_bytes())
            .expect("write gwtd request");
    }
    let deadline = Instant::now() + PHASE24_OPERATION_TIMEOUT;
    let status = loop {
        match child
            .0
            .as_mut()
            .expect("guarded gwtd child")
            .try_wait()
            .expect("poll short-lived gwtd")
        {
            Some(status) => break status,
            None if Instant::now() < deadline => thread::sleep(PHASE24_POLL_INTERVAL),
            None => {
                let (kill, reap) =
                    phase24_kill_and_reap_child(child.0.as_mut().expect("guarded gwtd child"));
                let stdout = phase24_read_capture(&mut stdout_capture);
                let stderr = phase24_read_capture(&mut stderr_capture);
                child.0.take();
                panic!(
                    "short-lived gwtd exceeded {:?}; kill={kill:?}; reap={reap:?}; stdout={stdout}; stderr={stderr}",
                    PHASE24_OPERATION_TIMEOUT
                );
            }
        }
    };
    child.0.take();
    let stdout = phase24_read_capture(&mut stdout_capture);
    let stderr = phase24_read_capture(&mut stderr_capture);
    let envelope: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "gwtd response is not one JSON object: {error}; stdout={}; stderr={}",
            stdout, stderr,
        )
    });
    let response = envelope["output"]
        .as_str()
        .and_then(|body| serde_json::from_str(body.trim()).ok())
        .unwrap_or_else(|| {
            panic!(
                "gwtd command output is not one JSON object: envelope={envelope}; stderr={}",
                stderr,
            )
        });
    (status, response)
}

#[cfg(unix)]
fn phase24_spawn_stale_status_daemon(
    project: &std::path::Path,
    stale_status: serde_json::Value,
) -> Phase24BoundedThread<()> {
    let scope = RuntimeScope::from_project_root(project, RuntimeTarget::Host)
        .expect("runtime scope for stale daemon");
    let socket_path = project.join("t252-stale-status.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind stale status endpoint");
    listener
        .set_nonblocking(true)
        .expect("make stale status listener nonblocking");
    let endpoint = DaemonEndpoint::new(
        scope.clone(),
        std::process::id(),
        socket_path.to_string_lossy().to_string(),
        "t252-stale-status-token".to_string(),
        "t252-stale-daemon".to_string(),
    );
    persist_endpoint(
        &scope.endpoint_path(&gwt_core::paths::gwt_home()),
        &endpoint,
    )
    .expect("persist stale resident endpoint");

    phase24_spawn_bounded_thread("stale status daemon", move || {
        let deadline = Instant::now() + PHASE24_OPERATION_TIMEOUT;
        let (stream, _) = loop {
            match listener.accept() {
                Ok(accepted) => break accepted,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "stale status daemon accept exceeded {PHASE24_OPERATION_TIMEOUT:?}"
                    );
                    thread::sleep(PHASE24_POLL_INTERVAL);
                }
                Err(error) => panic!("accept separate gwtd status: {error}"),
            }
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "stale status handshake deadline");
        stream
            .set_nonblocking(false)
            .expect("make accepted stale status stream blocking");
        stream
            .set_read_timeout(Some(remaining.min(PHASE24_IO_TIMEOUT)))
            .expect("bound stale status reads");
        stream
            .set_write_timeout(Some(remaining.min(PHASE24_IO_TIMEOUT)))
            .expect("bound stale status writes");
        let mut reader = BufReader::new(stream.try_clone().expect("clone status stream"));
        let mut writer = stream;
        let mut line = String::new();
        reader.read_line(&mut line).expect("read status handshake");
        let request: IpcHandshakeRequest =
            serde_json::from_str(line.trim_end()).expect("parse status handshake");
        assert_eq!(request.scope, scope);
        assert_eq!(request.auth_token, "t252-stale-status-token");
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "stale status handshake reply deadline"
        );
        writer
            .set_write_timeout(Some(remaining.min(PHASE24_IO_TIMEOUT)))
            .expect("rebound stale status handshake write");
        writeln!(
            writer,
            "{}",
            serde_json::to_string(&IpcHandshakeResponse {
                protocol_version: DAEMON_PROTOCOL_VERSION,
                daemon_version: "t252-stale-daemon".to_string(),
                accepted: true,
                rejection_reason: None,
            })
            .expect("serialize status handshake")
        )
        .expect("write status handshake");
        line.clear();
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "stale status frame deadline");
        reader
            .get_ref()
            .set_read_timeout(Some(remaining.min(PHASE24_IO_TIMEOUT)))
            .expect("rebound stale status frame read");
        reader.read_line(&mut line).expect("read status frame");
        assert_eq!(
            serde_json::from_str::<ClientFrame>(line.trim_end()).expect("parse status frame"),
            ClientFrame::Status
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "stale status reply deadline");
        writer
            .set_write_timeout(Some(remaining.min(PHASE24_IO_TIMEOUT)))
            .expect("rebound stale status reply write");
        writeln!(
            writer,
            "{}",
            serde_json::to_string(&DaemonFrame::Status(DaemonStatus {
                protocol_version: DAEMON_PROTOCOL_VERSION,
                daemon_version: "t252-stale-daemon".to_string(),
                uptime_seconds: 1,
                broadcast_channels: 0,
                connections: 1,
                issue_monitor: Some(stale_status),
            }))
            .expect("serialize stale daemon status")
        )
        .expect("write stale daemon status");
    })
}

fn phase24_control_params_from_status(
    status: &serde_json::Value,
    project: &std::path::Path,
    operation_id: &str,
) -> serde_json::Value {
    let issue = status["pm_control"]["issues"]
        .as_array()
        .and_then(|issues| issues.iter().find(|issue| issue["issue_number"] == 42))
        .expect("status exposes Issue 42 PM-control projection");
    let target = &issue["target"];
    assert_eq!(target["generation"], 1);
    assert_eq!(target["launched_window_id"], "tab-1::agent-42");
    serde_json::json!({
        "project_root": project,
        "number": 42,
        "operation_id": operation_id,
        "reason": "stalled provider",
        "launch_generation": target["generation"],
        "claim_id": target["claim_id"],
        "claim_owner": target["claim_owner"],
        "delivery_id": target["delivery_id"],
        "materializer_window_id": target["materializer_window_id"],
        "window_id": target["launched_window_id"],
        "launch_profile_fingerprint": target["source_profile_fingerprint"],
    })
}

fn phase24_seed_pending_reconcile(project: &std::path::Path) -> gwt::IssueMonitorPmControlTarget {
    let mut monitor = phase24_generated_launch();
    let target = monitor
        .pm_control_status(42)
        .target
        .expect("generated reconcile target");
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 42,
    };
    let execution_target = gwt::cli::execution_state::current_execution_binding(project, owner)
        .ok()
        .flatten()
        .and_then(|binding| {
            let record = gwt::cli::execution_state::load(project).ok().flatten()?;
            let session = gwt_agent::Session::load(
                &gwt_core::paths::gwt_sessions_dir()
                    .join(format!("{}.toml", record.primary_session_id)),
            )
            .ok()?;
            let session_identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
                .ok()
                .flatten()?;
            let session_binding = session_identity.execution_binding.clone();
            if session_binding.identity != binding {
                return None;
            }
            Some(gwt::IssueMonitorPmControlExecutionTarget {
                worktree_path: std::fs::canonicalize(project).ok()?,
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                generation_id: binding.generation_id,
                session_id: session_identity.session_id.clone(),
                session_binding,
                session_identity: Some(session_identity),
                settlement_operation_id:
                    "issue-monitor-control:recover-reconcile-e2e:exact-absence".to_string(),
            })
        });
    monitor
        .admit_pm_control_with_execution_target(
            gwt::IssueMonitorPmControlRequest {
                operation_id: "recover-reconcile-e2e".to_string(),
                project_key: gwt_core::paths::project_scope_hash(project)
                    .as_str()
                    .to_string(),
                action: gwt::IssueMonitorPmControlAction::Recover,
                actor_session: "pm-reconcile".to_string(),
                actor_registration_generation: 1,
                target: target.clone(),
                reason: "lost close acknowledgement".to_string(),
                requested_profile: Some(phase24_launch_profile("codex", "gpt-source")),
                admitted_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                admission_inventory_revision: 40,
            },
            execution_target,
        )
        .expect("seed pending reconcile");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(project),
        &monitor.prefs(),
    )
    .expect("persist pending reconcile");
    target
}

fn phase24_init_git_project(project: &Path) {
    for args in [
        vec!["init", "-b", "develop"],
        vec!["config", "user.email", "phase24@example.invalid"],
        vec!["config", "user.name", "Phase 24"],
        vec![
            "remote",
            "add",
            "origin",
            "https://example.invalid/phase24.git",
        ],
    ] {
        let output = gwt_core::process::run_git_logged(&args, Some(project))
            .expect("run Phase 24 git fixture command");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn phase24_seed_bound_ecr_generation(
    project: &Path,
) -> gwt::cli::execution_state::ExecutionOwnerKey {
    phase24_seed_bound_ecr_generation_with_session(project, "session-ecr-42")
}

fn phase24_seed_bound_ecr_generation_with_session(
    project: &Path,
    session_id: &str,
) -> gwt::cli::execution_state::ExecutionOwnerKey {
    phase24_init_git_project(project);
    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
        kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
        number: 42,
    };
    gwt::cli::execution_state::materialize_at_launch(
        project,
        owner.kind,
        owner.number,
        session_id,
        "gwt-execute",
        false,
    )
    .expect("seed active ECR");
    gwt::cli::execution_state::ensure_generation_ledger(
        project,
        owner,
        gwt::cli::execution_state::LegacyActiveDisposition::Live,
    )
    .expect("seed active ECR generation");
    let binding = gwt::cli::execution_state::current_execution_binding(project, owner)
        .expect("load current ECR binding")
        .expect("active ECR has a binding");
    let mut session = gwt_agent::Session::new(project, "develop", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(project.to_path_buf());
    session.linked_issue_number = Some(42);
    session
        .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: session.repo_hash.clone().expect("fixture repo hash"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: binding,
            capability_generation: 1,
        }))
        .expect("bind fixture Session to ECR");
    session
        .save(&gwt_core::paths::gwt_sessions_dir())
        .expect("persist exact ECR Session");
    owner
}

fn phase24_snapshot_file_tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, current: &Path, snapshot: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries = std::fs::read_dir(current)
            .unwrap_or_else(|error| panic!("read snapshot tree {}: {error}", current.display()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|error| {
                panic!("enumerate snapshot tree {}: {error}", current.display())
            });
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if entry
                .file_type()
                .unwrap_or_else(|error| {
                    panic!("inspect snapshot entry {}: {error}", path.display())
                })
                .is_dir()
            {
                visit(root, &path, snapshot);
            } else {
                snapshot.push((
                    path.strip_prefix(root)
                        .expect("snapshot path stays rooted")
                        .to_path_buf(),
                    std::fs::read(&path).unwrap_or_else(|error| {
                        panic!("read snapshot entry {}: {error}", path.display())
                    }),
                ));
            }
        }
    }

    let mut snapshot = Vec::new();
    visit(root, root, &mut snapshot);
    snapshot
}

fn phase24_snapshot_ecr_authority(project: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let trusted = gwt::cli::trusted_store::trusted_dir_for_worktree(project)
        .expect("git fixture has a trusted ECR directory");
    let mut snapshot = phase24_snapshot_file_tree(&trusted)
        .into_iter()
        .map(|(path, bytes)| (PathBuf::from("trusted").join(path), bytes))
        .collect::<Vec<_>>();
    for relative in [
        gwt::cli::execution_state::EXECUTION_CONTROL_STATE_RELATIVE,
        gwt::cli::execution_state::EXECUTION_GENERATION_POINTER_STATE_RELATIVE,
    ] {
        let path = project.join(relative);
        if path.exists() {
            snapshot.push((
                PathBuf::from("mirror").join(relative),
                std::fs::read(path).unwrap(),
            ));
        }
    }
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    snapshot
}

#[derive(Clone, Copy)]
enum Phase24InventoryReply {
    Absent,
    Present,
    Unavailable,
    Ambiguous,
    Stale,
    Foreign,
    ReplacementB(u64),
}

struct Phase24InventoryBarrier {
    query_received: std::sync::mpsc::Sender<()>,
    release_proof: std::sync::mpsc::Receiver<()>,
}

fn phase24_spawn_inventory_runtime(
    mode: Phase24InventoryReply,
    project: &std::path::Path,
    target: &gwt::IssueMonitorPmControlTarget,
    barrier: Option<Phase24InventoryBarrier>,
) -> (
    String,
    Phase24BoundedThread<()>,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind inventory runtime");
    listener
        .set_nonblocking(true)
        .expect("set inventory listener nonblocking");
    let address = listener.local_addr().expect("inventory runtime address");
    let project_key = gwt_core::paths::project_scope_hash(project)
        .as_str()
        .to_string();
    let target = target.clone();
    let connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_connections = std::sync::Arc::clone(&connections);
    let server = phase24_spawn_bounded_thread("inventory WebSocket runtime", move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build inventory runtime");
        runtime.block_on(async move {
            use futures_util::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::Message;

            tokio::time::timeout(PHASE24_OPERATION_TIMEOUT, async move {
                let listener =
                    tokio::net::TcpListener::from_std(listener).expect("adopt inventory listener");
                let (stream, _) =
                    match tokio::time::timeout(PHASE24_IO_TIMEOUT, listener.accept()).await {
                        Ok(Ok(accepted)) => accepted,
                        Ok(Err(error)) => panic!("accept inventory WebSocket: {error}"),
                        Err(_) => return,
                    };
                server_connections.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut socket = tokio::time::timeout(
                    PHASE24_IO_TIMEOUT,
                    tokio_tungstenite::accept_hdr_async(
                        stream,
                        |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                         response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                            assert_eq!(
                                request
                                    .headers()
                                    .get("authorization")
                                    .and_then(|value| value.to_str().ok()),
                                Some("Bearer t252-runtime-capability")
                            );
                            Ok(response)
                        },
                    ),
                )
                .await
                .expect("inventory WebSocket handshake is bounded")
                .expect("accept authenticated inventory websocket");
                let query = tokio::time::timeout(PHASE24_IO_TIMEOUT, async {
                    loop {
                        let message = socket
                            .next()
                            .await
                            .expect("inventory query frame")
                            .expect("valid inventory query frame");
                        let Message::Text(text) = message else {
                            continue;
                        };
                        let value: serde_json::Value =
                            serde_json::from_str(text.as_ref()).expect("inventory query JSON");
                        if value["kind"] == "agent_issue_monitor_runtime_inventory_proof" {
                            break value;
                        }
                    }
                })
                .await
                .expect("inventory query is bounded");
                assert_eq!(query["operation_id"], "recover-reconcile-e2e");
                assert_eq!(query["revoked_generation"], target.generation);
                assert_eq!(query["expected_project_scope"], project_key);
                assert!(query.get("absent").is_none());
                assert!(query.get("proof").is_none());
                if let Some(barrier) = barrier {
                    barrier
                        .query_received
                        .send(())
                        .expect("signal authenticated inventory query");
                    tokio::time::timeout(
                        PHASE24_IO_TIMEOUT,
                        tokio::task::spawn_blocking(move || {
                            barrier.release_proof.recv_timeout(PHASE24_IO_TIMEOUT)
                        }),
                    )
                    .await
                    .expect("inventory proof barrier task is bounded")
                    .expect("inventory proof barrier task joins")
                    .expect("wait to release inventory proof");
                }
                let request_id = query["request_id"].clone();
                let reply = {
                    let observation = match mode {
                        Phase24InventoryReply::Present => "present",
                        Phase24InventoryReply::Unavailable => "unavailable",
                        Phase24InventoryReply::Ambiguous => "ambiguous",
                        Phase24InventoryReply::Absent
                        | Phase24InventoryReply::Stale
                        | Phase24InventoryReply::Foreign
                        | Phase24InventoryReply::ReplacementB(_) => "absent",
                    };
                    let inventory_revision = match mode {
                        Phase24InventoryReply::Stale => 40,
                        Phase24InventoryReply::ReplacementB(revision) => revision,
                        _ => 41,
                    };
                    let proof_project_key = if matches!(mode, Phase24InventoryReply::Foreign) {
                        "foreign-project-key".to_string()
                    } else {
                        project_key
                    };
                    let runtime_id = if matches!(mode, Phase24InventoryReply::ReplacementB(_)) {
                        "app-runtime-b"
                    } else {
                        "app-runtime-a"
                    };
                    serde_json::json!({
                        "kind": "issue_monitor_runtime_inventory_proof",
                        "request_id": request_id,
                        "proof": {
                            "author": {
                                "kind": "owning_app_runtime",
                                "runtime_id": runtime_id,
                            },
                            "project_key": proof_project_key,
                            "inventory_revision": inventory_revision,
                            "generation": target.generation,
                            "materializer_id": target.materializer_id,
                            "materializer_pid": target.materializer_pid,
                            "window_id": target.materializer_window_id,
                            "observation": observation,
                            "observed_at": "2026-08-15T00:15:00Z",
                        },
                    })
                };
                tokio::time::timeout(
                    PHASE24_IO_TIMEOUT,
                    socket.send(Message::Text(reply.to_string().into())),
                )
                .await
                .expect("inventory proof send is bounded")
                .expect("send inventory result");
            })
            .await
            .expect("inventory WebSocket session is globally bounded");
        });
    });
    (
        format!("ws://{address}/internal/pane-ws"),
        server,
        connections,
    )
}

fn phase24_admit_pending_reconcile_via_gwtd(
    project: &Path,
    home: &Path,
) -> gwt::IssueMonitorPmControlTarget {
    let monitor = phase24_generated_launch();
    let target = monitor
        .pm_control_status(42)
        .target
        .expect("generated reconcile target");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(project),
        &monitor.prefs(),
    )
    .expect("seed generated launch before production admission");
    let (code, response) = phase24_run_gwtd(
        project,
        home,
        "pm-reconcile",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.recover",
            "params": {
                "project_root": project,
                "number": 42,
                "operation_id": "recover-reconcile-e2e",
                "reason": "lost close acknowledgement",
                "launch_generation": target.generation,
                "claim_id": target.claim_id,
                "claim_owner": target.claim_owner,
                "delivery_id": target.delivery_id,
                "materializer_window_id": target.materializer_window_id,
                "window_id": target.launched_window_id,
                "launch_profile_fingerprint": target.source_profile_fingerprint,
            },
        }),
    );
    assert!(code.success(), "production admission failed: {response}");
    assert_eq!(response["outcome"], "teardown_pending");
    target
}

fn phase24_persist_successor_session(
    project: &Path,
    owner: gwt::cli::execution_state::ExecutionOwnerKey,
    session_id: &str,
) -> PathBuf {
    let binding = gwt::cli::execution_state::current_execution_binding(project, owner)
        .expect("load successor ECR binding")
        .expect("successor ECR has a binding");
    let mut session = gwt_agent::Session::new(project, "develop", gwt_agent::AgentId::Codex);
    session.id = session_id.to_string();
    session.project_state_root = Some(project.to_path_buf());
    session.linked_issue_number = Some(42);
    session
        .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: session.repo_hash.clone().expect("fixture repo hash"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: binding,
            capability_generation: 1,
        }))
        .expect("bind successor Session to ECR");
    session
        .save(&gwt_core::paths::gwt_sessions_dir())
        .expect("persist successor ECR Session");
    gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"))
}

#[cfg(unix)]
fn phase24_spawn_status_inventory_runtime(
    project: &Path,
    window_id: &str,
) -> (String, Phase24BoundedThread<()>) {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind status inventory runtime");
    listener
        .set_nonblocking(true)
        .expect("set status inventory listener nonblocking");
    let address = listener.local_addr().expect("status inventory address");
    let project = project.to_path_buf();
    let window_id = window_id.to_string();
    let server = phase24_spawn_bounded_thread("status inventory WebSocket runtime", move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build status inventory runtime");
        runtime.block_on(async move {
            use futures_util::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::Message;

            tokio::time::timeout(PHASE24_OPERATION_TIMEOUT, async move {
                let listener = tokio::net::TcpListener::from_std(listener)
                    .expect("adopt status inventory listener");
                let (stream, _) =
                    match tokio::time::timeout(PHASE24_IO_TIMEOUT, listener.accept()).await {
                        Ok(Ok(accepted)) => accepted,
                        Ok(Err(error)) => panic!("accept status inventory WebSocket: {error}"),
                        Err(_) => return,
                    };
                let mut socket = tokio::time::timeout(
                    PHASE24_IO_TIMEOUT,
                    tokio_tungstenite::accept_hdr_async(
                        stream,
                        |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                         response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                            assert_eq!(
                                request
                                    .headers()
                                    .get("authorization")
                                    .and_then(|value| value.to_str().ok()),
                                Some("Bearer t496-status-runtime-capability")
                            );
                            Ok(response)
                        },
                    ),
                )
                .await
                .expect("status inventory WebSocket handshake is bounded")
                .expect("accept authenticated status inventory websocket");
                let query = loop {
                    let message = socket
                        .next()
                        .await
                        .expect("status inventory frontend frame")
                        .expect("valid status inventory frontend frame");
                    let Message::Text(text) = message else {
                        continue;
                    };
                    let value: serde_json::Value = serde_json::from_str(text.as_ref())
                        .expect("status inventory frontend JSON");
                    if value["kind"] == "agent_issue_monitor_runtime_inventory" {
                        break value;
                    }
                };
                assert_eq!(
                    query["expected_project_scope"],
                    gwt_core::paths::project_scope_hash(&project).as_str()
                );
                let request_id = query["request_id"]
                    .as_str()
                    .expect("status inventory request id")
                    .to_string();
                let reply = serde_json::json!({
                    "kind": "issue_monitor_runtime_inventory",
                    "request_id": request_id,
                    "inventory": {
                        "availability": "available",
                        "project_scope": gwt_core::paths::project_scope_hash(&project).as_str(),
                        "runtime_instance_id": "app-runtime-status-a",
                        "revision": 73,
                        "observed_at": "2026-08-29T12:00:00Z",
                        "windows": [{
                            "window_id": window_id,
                            "pane_state": "running",
                            "wait_signal": null
                        }]
                    }
                });
                socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .expect("send authenticated status inventory");
            })
            .await
            .expect("status inventory runtime obeys one total deadline");
        });
    });
    (format!("ws://{address}/internal/pane-ws"), server)
}

/// SPEC-3431 Phase 24R / T496: `issue.monitor.status` must not rely on a
/// resident JSON field that no production publisher authors. The public CLI
/// establishes the authenticated AppRuntime connection itself and overlays
/// that live pane inventory onto the durable control projection.
#[test]
#[cfg(unix)]
fn t496_status_queries_authenticated_owning_runtime_before_projecting_panes() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("status project tempdir");
    let home = TempDir::new().expect("status home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let monitor = phase24_generated_launch();
    gwt_github::Cache::new(
        gwt::issue_cache::issue_cache_root_for_repo_path_or_detached(project.path()),
    )
    .write_snapshot(&gwt_github::IssueSnapshot {
        number: gwt_github::IssueNumber(42),
        title: "Issue 42".to_string(),
        body: String::new(),
        labels: vec!["auto-merge".to_string()],
        state: gwt_github::IssueState::Open,
        updated_at: gwt_github::UpdatedAt::new("2026-08-29T11:59:00Z"),
        comments: Vec::new(),
    })
    .expect("seed status issue cache");
    gwt::save_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(project.path()),
        &monitor.prefs(),
    )
    .expect("seed status control projection");
    let (ws_url, runtime) =
        phase24_spawn_status_inventory_runtime(project.path(), "tab-1::agent-42");
    let _pane_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &ws_url);
    let _pane_token = ScopedEnvVar::set(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
        "t496-status-runtime-capability",
    );

    let (code, status) = phase24_run_gwtd(
        project.path(),
        home.path(),
        "pm-status",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.status",
            "params": { "project_root": project.path() },
        }),
    );
    runtime.join().expect("status inventory runtime exits");

    assert!(code.success(), "status query failed: {status}");
    assert_eq!(status["runtime_inventory"]["availability"], "available");
    let row = status["inbox"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["issue_number"] == 42))
        .expect("status exposes Issue 42");
    assert_eq!(row["pane_state"], "running");
    assert_eq!(row["runtime_consistency"], "consistent");
    assert_eq!(
        row["control_ready"]["recover"]["degraded_reason"],
        "runtime_live"
    );
}

/// SPEC-3431 AS-PM-CONTROL-EXECUTION-SETTLEMENT-001: an authoritative
/// absence is not permission to erase Monitor identity while its producing
/// ECR generation still says Active. The exact generation is terminalized
/// first and the Monitor commit follows only after the readback fence.
#[test]
#[cfg(unix)]
fn t496_exact_absence_settles_matching_ecr_generation_before_monitor_cleanup() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("ECR project tempdir");
    let home = TempDir::new().expect("ECR home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let owner = phase24_seed_bound_ecr_generation(project.path());
    phase24_register_pm(project.path(), "pm-reconcile");
    let target = phase24_seed_pending_reconcile(project.path());
    let (ws_url, runtime, _) = phase24_spawn_inventory_runtime(
        Phase24InventoryReply::Absent,
        project.path(),
        &target,
        None,
    );
    let _pane_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &ws_url);
    let _pane_token = ScopedEnvVar::set(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
        "t252-runtime-capability",
    );

    let (code, response) = phase24_run_gwtd(
        project.path(),
        home.path(),
        "pm-reconcile",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.control.reconcile",
            "params": {
                "project_root": project.path(),
                "operation_id": "recover-reconcile-e2e",
                "revoked_generation": target.generation,
            },
        }),
    );
    runtime.join().expect("absence runtime exits");

    assert!(code.success(), "exact absence reconcile failed: {response}");
    let ledger = gwt::cli::execution_state::load_generation_ledger(project.path(), owner)
        .expect("load reconciled ECR generation")
        .expect("ECR generation remains durable");
    assert_eq!(
        ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked),
        "Monitor cleanup cannot lead an Active producing generation"
    );
    let prefs =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(project.path()))
            .expect("load reconciled Monitor prefs");
    assert!(prefs
        .pm_control
        .issues
        .get(&42)
        .and_then(|state| state.pending_control.as_ref())
        .is_some_and(|pending| pending.teardown_settled));
}

/// A crash can land after the exact ECR lifecycle commit and after ordinary
/// Session cleanup but before Monitor commit. The admission-bound full
/// Session identity is therefore the replay authority; the missing TOML is
/// accepted only for the exact existing lifecycle receipt.
#[test]
#[cfg(unix)]
fn t496_ecr_first_replay_survives_exact_session_cleanup_before_monitor_commit() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("missing Session replay project tempdir");
    let home = TempDir::new().expect("missing Session replay home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let owner = phase24_seed_bound_ecr_generation(project.path());
    phase24_register_pm(project.path(), "pm-reconcile");
    let target = phase24_admit_pending_reconcile_via_gwtd(project.path(), home.path());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project.path());
    let admitted = gwt::load_issue_monitor_prefs(&prefs_path).expect("load exact admission");
    let execution_target = admitted
        .pm_control
        .issues
        .get(&42)
        .and_then(|state| state.pending_control.as_ref())
        .and_then(|pending| pending.execution_target.as_ref())
        .expect("admission-bound execution target");
    let identity = execution_target
        .session_identity
        .as_ref()
        .expect("new admission stores full Session identity")
        .clone();
    gwt::cli::execution_state::settle_exact_absent_active_generation(
        project.path(),
        owner,
        &execution_target.settlement_operation_id,
        &gwt_core::paths::gwt_sessions_dir(),
        &identity,
        "owning AppRuntime proved the Issue Monitor launch absent",
    )
    .expect("simulate ECR-first crash point");
    std::fs::remove_file(
        gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", identity.session_id)),
    )
    .expect("simulate exact Session cleanup after ECR commit");
    let before = std::fs::read(&prefs_path).expect("pending Monitor bytes before replay");
    let (ws_url, runtime, _) = phase24_spawn_inventory_runtime(
        Phase24InventoryReply::Absent,
        project.path(),
        &target,
        None,
    );
    let _pane_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &ws_url);
    let _pane_token = ScopedEnvVar::set(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
        "t252-runtime-capability",
    );
    let (code, response) = phase24_run_gwtd(
        project.path(),
        home.path(),
        "pm-reconcile",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.control.reconcile",
            "params": {
                "project_root": project.path(),
                "operation_id": "recover-reconcile-e2e",
                "revoked_generation": target.generation,
            },
        }),
    );
    runtime
        .join()
        .expect("missing Session replay runtime exits");
    assert!(code.success(), "missing Session replay failed: {response}");
    assert_ne!(
        std::fs::read(&prefs_path).expect("Monitor bytes after replay"),
        before
    );
    assert!(gwt::load_issue_monitor_prefs(&prefs_path)
        .unwrap()
        .pm_control
        .issues
        .get(&42)
        .and_then(|state| state.pending_control.as_ref())
        .is_some_and(|pending| pending.teardown_settled));
}

#[test]
#[cfg(unix)]
fn t496_legacy_execution_target_without_full_session_identity_fails_closed() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("legacy execution target project tempdir");
    let home = TempDir::new().expect("legacy execution target home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    phase24_seed_bound_ecr_generation(project.path());
    phase24_register_pm(project.path(), "pm-reconcile");
    let target = phase24_admit_pending_reconcile_via_gwtd(project.path(), home.path());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project.path());
    let mut prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load exact admission");
    prefs
        .pm_control
        .issues
        .get_mut(&42)
        .and_then(|state| state.pending_control.as_mut())
        .and_then(|pending| pending.execution_target.as_mut())
        .expect("pending exact target")
        .session_identity = None;
    prefs
        .pm_control
        .receipts
        .iter_mut()
        .find(|receipt| receipt.operation_id == "recover-reconcile-e2e")
        .and_then(|receipt| receipt.execution_target.as_mut())
        .expect("receipt exact target")
        .session_identity = None;
    gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("persist legacy target fixture");
    let before = std::fs::read(&prefs_path).expect("legacy target bytes before reconcile");
    let (ws_url, runtime, _) = phase24_spawn_inventory_runtime(
        Phase24InventoryReply::Absent,
        project.path(),
        &target,
        None,
    );
    let _pane_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &ws_url);
    let _pane_token = ScopedEnvVar::set(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
        "t252-runtime-capability",
    );
    let (code, response) = phase24_run_gwtd(
        project.path(),
        home.path(),
        "pm-reconcile",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.control.reconcile",
            "params": {
                "project_root": project.path(),
                "operation_id": "recover-reconcile-e2e",
                "revoked_generation": target.generation,
            },
        }),
    );
    runtime.join().expect("legacy target runtime exits");
    assert!(!code.success());
    assert_eq!(
        response,
        serde_json::json!({
            "status": "refused",
            "error": "execution_settlement_unknown"
        })
    );
    assert_eq!(
        std::fs::read(&prefs_path).expect("legacy target bytes after reconcile"),
        before
    );
}

/// The absence proof is only evidence, not a cleanup override. If the
/// matching ECR generation cannot be identified exactly, reconciliation is a
/// byte-identical refusal and leaves requeue/release work fenced.
#[test]
#[cfg(unix)]
fn t496_exact_absence_with_unknown_ecr_refuses_without_monitor_mutation() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("unknown ECR project tempdir");
    let home = TempDir::new().expect("unknown ECR home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    phase24_init_git_project(project.path());
    phase24_register_pm(project.path(), "pm-reconcile");
    let target = phase24_seed_pending_reconcile(project.path());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project.path());
    let before = std::fs::read(&prefs_path).expect("prefs before unknown-ECR reconcile");
    let (ws_url, runtime, _) = phase24_spawn_inventory_runtime(
        Phase24InventoryReply::Absent,
        project.path(),
        &target,
        None,
    );
    let _pane_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &ws_url);
    let _pane_token = ScopedEnvVar::set(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
        "t252-runtime-capability",
    );

    let (code, response) = phase24_run_gwtd(
        project.path(),
        home.path(),
        "pm-reconcile",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.control.reconcile",
            "params": {
                "project_root": project.path(),
                "operation_id": "recover-reconcile-e2e",
                "revoked_generation": target.generation,
            },
        }),
    );
    runtime.join().expect("unknown ECR runtime exits");

    assert!(!code.success(), "unknown ECR was accepted: {response}");
    assert_eq!(
        response,
        serde_json::json!({
            "status": "refused",
            "error": "execution_settlement_unknown"
        })
    );
    assert_eq!(
        std::fs::read(&prefs_path).expect("prefs after unknown-ECR reconcile"),
        before,
        "unknown ECR settlement must be Monitor-byte-identical"
    );
}

/// The ECR-first saga must not consume a settlement receipt after another
/// exact launch has already advanced that owner. Hold the Monitor writer lock
/// until the first generation is Blocked, activate a canonical successor,
/// then let reconcile resume. The successor and pending Monitor control must
/// both remain byte-for-byte/identity identical to the raced state.
#[test]
#[cfg(unix)]
fn t496_successor_between_ecr_settlement_and_monitor_commit_refuses_stale_receipt() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("successor race project tempdir");
    let home = TempDir::new().expect("successor race home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let owner = phase24_seed_bound_ecr_generation(project.path());
    phase24_register_pm(project.path(), "pm-reconcile");
    let target = phase24_seed_pending_reconcile(project.path());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project.path());
    let prefs_before = std::fs::read(&prefs_path).expect("prefs before successor race");
    let prefs_lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(prefs_path.with_extension("lock"))
        .expect("open Monitor prefs lock");
    prefs_lock
        .lock_exclusive()
        .expect("hold Monitor commit behind successor race");

    let (ws_url, runtime, _) = phase24_spawn_inventory_runtime(
        Phase24InventoryReply::Absent,
        project.path(),
        &target,
        None,
    );
    let _pane_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &ws_url);
    let _pane_token = ScopedEnvVar::set(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
        "t252-runtime-capability",
    );
    let race_project = project.path().to_path_buf();
    let race_home = home.path().to_path_buf();
    let child = phase24_spawn_bounded_thread("successor receipt race gwtd", move || {
        let _gwt_home = ScopedGwtHome::set(&race_home);
        phase24_run_gwtd(
            &race_project,
            &race_home,
            "pm-reconcile",
            serde_json::json!({
                "schema_version": 1,
                "operation": "issue.monitor.control.reconcile",
                "params": {
                    "project_root": race_project,
                    "operation_id": "recover-reconcile-e2e",
                    "revoked_generation": target.generation,
                },
            }),
        )
    });
    runtime.join().expect("successor race runtime exits");

    let deadline = Instant::now() + PHASE24_OPERATION_TIMEOUT;
    loop {
        let settled = gwt::cli::execution_state::load_generation_ledger(project.path(), owner)
            .expect("poll ECR settlement")
            .is_some_and(|ledger| {
                ledger.current_effective_status()
                    == Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked)
            });
        if settled {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "ECR settlement did not reach the Monitor commit barrier"
        );
        thread::sleep(PHASE24_POLL_INTERVAL);
    }
    assert_eq!(
        std::fs::read(&prefs_path).expect("prefs while Monitor commit is blocked"),
        prefs_before,
        "ECR settlement must lead the Monitor mutation"
    );

    let successor_request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "t496-successor-after-absence".to_string(),
        principal_id: "t496-racing-launch".to_string(),
        work_id: None,
        source: gwt::cli::execution_state::FRESH_LINKED_OWNER_LAUNCH_SOURCE.to_string(),
        session_binding_id: "t496-successor-binding".to_string(),
        initial_session_id: "t496-successor-session".to_string(),
        entrypoint: "gwt launch agent".to_string(),
        requested_at: chrono::Utc::now(),
    };
    gwt::cli::execution_state::prepare_fresh_linked_owner_launch_successor(
        project.path(),
        owner,
        &successor_request,
    )
    .expect("prepare raced successor");
    gwt::cli::execution_state::activate_successor(project.path(), owner, &successor_request)
        .expect("activate raced successor");
    let successor_ledger = gwt::cli::execution_state::load_generation_ledger(project.path(), owner)
        .expect("load raced successor")
        .expect("successor ledger remains durable");

    FileExt::unlock(&prefs_lock).expect("release Monitor commit barrier");
    let (code, response) = child.join().expect("successor receipt race gwtd exits");
    assert!(
        !code.success(),
        "stale ECR receipt was accepted: {response}"
    );
    assert_eq!(
        response,
        serde_json::json!({
            "status": "refused",
            "error": "execution_settlement_unknown"
        })
    );
    assert_eq!(
        std::fs::read(&prefs_path).expect("prefs after successor receipt refusal"),
        prefs_before,
        "stale ECR receipt must not settle or replace Monitor state"
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(project.path(), owner)
            .expect("reload raced successor")
            .expect("raced successor remains durable"),
        successor_ledger,
        "reconcile must not delete, replace, or requeue over the raced successor"
    );
}

/// A new Recover operation is admitted only when exactly one Active ECR and
/// Session binding can be copied into its durable pending/receipt identity.
/// Missing or same-project ambiguous candidates refuse before any Monitor
/// bytes are written.
#[test]
#[cfg(unix)]
fn t496_recover_admission_refuses_missing_or_ambiguous_execution_target_byte_identically() {
    let _env_lock = env_test_lock();
    let home = TempDir::new().expect("execution target admission home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());

    let missing = TempDir::new().expect("missing execution target project");
    phase24_register_pm(missing.path(), "pm-reconcile");
    let missing_monitor = phase24_generated_launch();
    let missing_target = missing_monitor
        .pm_control_status(42)
        .target
        .expect("missing-target Monitor identity");
    let missing_prefs = gwt::issue_monitor_prefs_path_for_repo_path(missing.path());
    gwt::save_issue_monitor_prefs(&missing_prefs, &missing_monitor.prefs())
        .expect("persist missing-target Monitor prefs");
    let missing_before = std::fs::read(&missing_prefs).expect("missing-target prefs before admit");
    let (missing_code, missing_response) = phase24_run_gwtd(
        missing.path(),
        home.path(),
        "pm-reconcile",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.recover",
            "params": {
                "project_root": missing.path(),
                "number": 42,
                "operation_id": "recover-reconcile-e2e",
                "reason": "lost close acknowledgement",
                "launch_generation": missing_target.generation,
                "claim_id": missing_target.claim_id,
                "claim_owner": missing_target.claim_owner,
                "delivery_id": missing_target.delivery_id,
                "materializer_window_id": missing_target.materializer_window_id,
                "window_id": missing_target.launched_window_id,
                "launch_profile_fingerprint": missing_target.source_profile_fingerprint,
            },
        }),
    );
    assert!(!missing_code.success());
    assert_eq!(
        missing_response,
        serde_json::json!({"status":"refused","error":"execution_target_unavailable"})
    );
    assert_eq!(
        std::fs::read(&missing_prefs).expect("missing-target prefs after refusal"),
        missing_before
    );

    let ambiguous_a = TempDir::new().expect("ambiguous execution target A");
    let ambiguous_b = TempDir::new().expect("ambiguous execution target B");
    phase24_seed_bound_ecr_generation_with_session(ambiguous_a.path(), "session-ecr-ambiguous-a");
    phase24_seed_bound_ecr_generation_with_session(ambiguous_b.path(), "session-ecr-ambiguous-b");
    assert_eq!(
        gwt_core::paths::project_scope_hash(ambiguous_a.path()),
        gwt_core::paths::project_scope_hash(ambiguous_b.path()),
        "fixture repositories must share one project scope"
    );
    phase24_register_pm(ambiguous_a.path(), "pm-reconcile");
    let ambiguous_monitor = phase24_generated_launch();
    let ambiguous_target = ambiguous_monitor
        .pm_control_status(42)
        .target
        .expect("ambiguous-target Monitor identity");
    let ambiguous_prefs = gwt::issue_monitor_prefs_path_for_repo_path(ambiguous_a.path());
    gwt::save_issue_monitor_prefs(&ambiguous_prefs, &ambiguous_monitor.prefs())
        .expect("persist ambiguous-target Monitor prefs");
    let ambiguous_before =
        std::fs::read(&ambiguous_prefs).expect("ambiguous-target prefs before admit");
    let (ambiguous_code, ambiguous_response) = phase24_run_gwtd(
        ambiguous_a.path(),
        home.path(),
        "pm-reconcile",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.recover",
            "params": {
                "project_root": ambiguous_a.path(),
                "number": 42,
                "operation_id": "recover-reconcile-e2e",
                "reason": "lost close acknowledgement",
                "launch_generation": ambiguous_target.generation,
                "claim_id": ambiguous_target.claim_id,
                "claim_owner": ambiguous_target.claim_owner,
                "delivery_id": ambiguous_target.delivery_id,
                "materializer_window_id": ambiguous_target.materializer_window_id,
                "window_id": ambiguous_target.launched_window_id,
                "launch_profile_fingerprint": ambiguous_target.source_profile_fingerprint,
            },
        }),
    );
    assert!(!ambiguous_code.success());
    assert_eq!(
        ambiguous_response,
        serde_json::json!({"status":"refused","error":"execution_target_unavailable"})
    );
    assert_eq!(
        std::fs::read(&ambiguous_prefs).expect("ambiguous-target prefs after refusal"),
        ambiguous_before
    );
}

/// An authenticated replacement AppRuntime is honest but not yet ordered
/// after the materializer that admitted the control. Its first absence proof
/// durably establishes only the replacement revision fence. A byte-identical
/// replay remains inert; only a strictly newer proof may consume the exact ECR
/// settlement target and advance Monitor teardown.
#[test]
#[cfg(unix)]
fn t496_replacement_runtime_requires_durable_fence_then_strictly_newer_absence() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("replacement runtime project tempdir");
    let home = TempDir::new().expect("replacement runtime home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let owner = phase24_seed_bound_ecr_generation(project.path());
    phase24_register_pm(project.path(), "pm-reconcile");
    let target = phase24_admit_pending_reconcile_via_gwtd(project.path(), home.path());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project.path());

    let run_proof = |reply| {
        let (ws_url, runtime, _) =
            phase24_spawn_inventory_runtime(reply, project.path(), &target, None);
        let _pane_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &ws_url);
        let _pane_token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "t252-runtime-capability",
        );
        let result = phase24_run_gwtd(
            project.path(),
            home.path(),
            "pm-reconcile",
            serde_json::json!({
                "schema_version": 1,
                "operation": "issue.monitor.control.reconcile",
                "params": {
                    "project_root": project.path(),
                    "operation_id": "recover-reconcile-e2e",
                    "revoked_generation": target.generation,
                },
            }),
        );
        runtime.join().expect("replacement runtime proof exits");
        result
    };

    let (first_code, first_response) = run_proof(Phase24InventoryReply::ReplacementB(50));
    assert!(
        !first_code.success(),
        "replacement runtime's fence-only proof settled teardown: {first_response}"
    );
    assert_eq!(
        first_response,
        serde_json::json!({
            "status": "refused",
            "error": "runtime_replacement_fenced"
        })
    );
    let first_ledger = gwt::cli::execution_state::load_generation_ledger(project.path(), owner)
        .expect("load ECR after replacement fence")
        .expect("ECR remains durable");
    assert_eq!(
        first_ledger.current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Active),
        "the first honest replacement proof establishes ordering but cannot settle ECR"
    );
    let first_prefs = gwt::load_issue_monitor_prefs(&prefs_path)
        .expect("load durable replacement inventory fence");
    let first_pending = first_prefs
        .pm_control
        .issues
        .get(&42)
        .and_then(|state| state.pending_control.as_ref())
        .expect("replacement fence keeps Monitor pending");
    assert!(
        first_pending
            .inventory_revision_fence
            .as_ref()
            .is_some_and(
                |fence| fence.runtime_id == "app-runtime-b" && fence.inventory_revision == 50
            )
    );
    assert!(!first_pending.teardown_settled);
    let first_bytes = std::fs::read(&prefs_path).expect("prefs after replacement fence");

    let (replay_code, replay_response) = run_proof(Phase24InventoryReply::ReplacementB(50));
    assert!(
        !replay_code.success(),
        "same-revision replacement replay succeeded: {replay_response}"
    );
    assert_eq!(
        replay_response,
        serde_json::json!({"status": "refused", "error": "inventory_stale"})
    );
    assert_eq!(
        std::fs::read(&prefs_path).expect("prefs after replacement replay"),
        first_bytes,
        "same runtime/revision replay must be byte-identical"
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(project.path(), owner)
            .expect("reload ECR after replacement replay")
            .expect("ECR remains durable"),
        first_ledger,
        "same-revision replay cannot mutate ECR"
    );

    let (newer_code, newer_response) = run_proof(Phase24InventoryReply::ReplacementB(51));
    assert!(
        newer_code.success(),
        "strictly newer replacement absence was refused: {newer_response}"
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(project.path(), owner)
            .expect("reload ECR after newer replacement proof")
            .expect("settled ECR remains durable")
            .current_effective_status(),
        Some(gwt::cli::execution_state::ExecutionControlStatus::Blocked)
    );
    let settled = gwt::load_issue_monitor_prefs(&prefs_path)
        .expect("load Monitor after newer replacement proof");
    assert!(
        settled
            .pm_control
            .issues
            .get(&42)
            .and_then(|state| state.pending_control.as_ref())
            .is_some_and(|pending| pending.teardown_settled),
        "strictly newer replacement proof settles Monitor after ECR"
    );
}

/// The reconcile target is the exact ECR/Session identity captured by the
/// production admission, not whichever generation happens to have an
/// `activated_at` value in the same serialized second. Advance a successor
/// while the owning runtime holds the proof barrier; the later absence must
/// leave every successor authority byte and Monitor byte untouched.
#[test]
#[cfg(unix)]
fn t496_same_second_successor_during_proof_cannot_retarget_admission_identity() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("same-second successor project tempdir");
    let home = TempDir::new().expect("same-second successor home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let owner = phase24_seed_bound_ecr_generation(project.path());
    phase24_register_pm(project.path(), "pm-reconcile");
    let target = phase24_admit_pending_reconcile_via_gwtd(project.path(), home.path());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project.path());
    let admitted = gwt::load_issue_monitor_prefs(&prefs_path).expect("load admitted Monitor prefs");
    let admitted_at = admitted
        .pm_control
        .issues
        .get(&42)
        .and_then(|state| state.pending_control.as_ref())
        .and_then(|pending| chrono::DateTime::parse_from_rfc3339(&pending.requested_at).ok())
        .expect("production admission timestamp")
        .with_timezone(&chrono::Utc);

    let (query_tx, query_rx) = std::sync::mpsc::channel();
    let (proof_tx, proof_rx) = std::sync::mpsc::channel();
    let (ws_url, runtime, _) = phase24_spawn_inventory_runtime(
        Phase24InventoryReply::Absent,
        project.path(),
        &target,
        Some(Phase24InventoryBarrier {
            query_received: query_tx,
            release_proof: proof_rx,
        }),
    );
    let _pane_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &ws_url);
    let _pane_token = ScopedEnvVar::set(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
        "t252-runtime-capability",
    );
    let race_project = project.path().to_path_buf();
    let race_home = home.path().to_path_buf();
    let child = phase24_spawn_bounded_thread("same-second successor gwtd", move || {
        let _gwt_home = ScopedGwtHome::set(&race_home);
        phase24_run_gwtd(
            &race_project,
            &race_home,
            "pm-reconcile",
            serde_json::json!({
                "schema_version": 1,
                "operation": "issue.monitor.control.reconcile",
                "params": {
                    "project_root": race_project,
                    "operation_id": "recover-reconcile-e2e",
                    "revoked_generation": target.generation,
                },
            }),
        )
    });
    query_rx
        .recv_timeout(PHASE24_OPERATION_TIMEOUT)
        .expect("proof query reaches same-second successor barrier");

    let successor_request = gwt::cli::execution_state::SuccessorRequest {
        operation_id: "t496-same-second-successor".to_string(),
        principal_id: "t496-racing-continuation".to_string(),
        work_id: None,
        source: "execution-continue".to_string(),
        session_binding_id: "t496-same-second-binding".to_string(),
        initial_session_id: "t496-same-second-session".to_string(),
        entrypoint: "continue-work".to_string(),
        requested_at: admitted_at,
    };
    gwt::cli::execution_state::prepare_active_continuation_successor(
        project.path(),
        owner,
        &successor_request,
    )
    .expect("prepare same-second successor");
    gwt::cli::execution_state::activate_successor(project.path(), owner, &successor_request)
        .expect("activate same-second successor");
    let successor_session_path =
        phase24_persist_successor_session(project.path(), owner, "t496-same-second-session");
    let successor_ledger = gwt::cli::execution_state::load_generation_ledger(project.path(), owner)
        .expect("load same-second successor ledger")
        .expect("same-second successor ledger remains durable");
    let successor_ecr_bytes = phase24_snapshot_ecr_authority(project.path());
    assert_eq!(
        successor_ledger
            .current_generation()
            .expect("same-second current generation")
            .identity
            .activated_at
            .timestamp(),
        admitted_at.timestamp(),
        "fixture must collide in the persisted admission second"
    );
    let successor_session_bytes =
        std::fs::read(&successor_session_path).expect("same-second successor Session bytes");
    let prefs_before_proof = std::fs::read(&prefs_path).expect("Monitor bytes before raced proof");

    proof_tx
        .send(())
        .expect("release same-second absence proof");
    runtime.join().expect("same-second runtime exits");
    let (code, response) = child.join().expect("same-second reconcile exits");
    assert!(
        !code.success(),
        "absence retargeted the same-second successor: {response}"
    );
    assert_eq!(
        gwt::cli::execution_state::load_generation_ledger(project.path(), owner)
            .expect("reload same-second successor ledger")
            .expect("same-second successor remains durable"),
        successor_ledger,
        "admission-bound reconciliation cannot mutate successor ECR"
    );
    assert_eq!(
        phase24_snapshot_ecr_authority(project.path()),
        successor_ecr_bytes,
        "admission-bound reconciliation preserves every successor ECR authority byte"
    );
    assert_eq!(
        std::fs::read(&successor_session_path).expect("successor Session after raced proof"),
        successor_session_bytes,
        "admission-bound reconciliation cannot mutate successor Session"
    );
    assert_eq!(
        std::fs::read(&prefs_path).expect("Monitor after raced proof"),
        prefs_before_proof,
        "a changed exact execution identity keeps Monitor pending byte-identically"
    );
}

/// SPEC-3431 Phase 24 / T252: status and control deliberately run in separate
/// processes. The status response is the only source of the exact target; a
/// PM replacement between those processes invalidates the old principal
/// without disclosing or mutating Issue Monitor state. The replacement PM can
/// re-read and submit that same durable identity successfully.
#[test]
fn t252_fresh_status_to_separate_gwtd_control_is_revisioned_and_pm_generation_bound() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("project tempdir");
    let home = TempDir::new().expect("home tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    assert_eq!(
        gwt_core::paths::gwt_home(),
        home.path().join(".gwt"),
        "the parent must use the child process's isolated <home>/.gwt"
    );
    phase24_seed_bound_ecr_generation(project.path());
    phase24_register_pm(project.path(), "pm-session-a");
    let monitor = phase24_generated_launch();
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project.path());
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed generated launch");

    let (status_code, observed) = phase24_run_gwtd(
        project.path(),
        home.path(),
        "pm-session-a",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.status",
            "params": { "project_root": project.path() },
        }),
    );
    assert!(status_code.success(), "fresh status failed: {observed}");
    assert!(
        observed["pm_control"]["revision"]
            .as_u64()
            .is_some_and(|revision| revision > 0),
        "status must expose the disk control-state revision: {observed}"
    );
    let old_pm_request =
        phase24_control_params_from_status(&observed, project.path(), "recover-cross-process");

    // Status returned and its gwtd process is gone. Replace the durable PM
    // principal before starting the second process.
    phase24_register_pm(project.path(), "pm-session-b");
    let before_refusal = std::fs::read(&prefs_path).expect("prefs before stale PM control");
    let (refused_code, refused) = phase24_run_gwtd(
        project.path(),
        home.path(),
        "pm-session-a",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.recover",
            "params": old_pm_request,
        }),
    );
    assert!(!refused_code.success(), "replaced PM succeeded: {refused}");
    assert_eq!(
        refused,
        serde_json::json!({"status": "refused", "error": "unauthorized"})
    );
    assert_eq!(
        std::fs::read(&prefs_path).expect("prefs after stale PM refusal"),
        before_refusal,
        "PM replacement refusal is byte-identical"
    );

    let (_, fresh) = phase24_run_gwtd(
        project.path(),
        home.path(),
        "pm-session-b",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.status",
            "params": { "project_root": project.path() },
        }),
    );
    let replacement_request =
        phase24_control_params_from_status(&fresh, project.path(), "recover-replacement-pm");
    let replacement_replay_request = replacement_request.clone();
    let (accepted_code, accepted) = phase24_run_gwtd(
        project.path(),
        home.path(),
        "pm-session-b",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.recover",
            "params": replacement_request,
        }),
    );
    assert!(
        accepted_code.success(),
        "replacement PM was refused: {accepted}"
    );
    assert_eq!(accepted["operation_id"], "recover-replacement-pm");
    assert_eq!(accepted["action"], "recover");
    assert_eq!(accepted["outcome"], "teardown_pending");
    assert_eq!(accepted["scan_requested"], false);
    assert_eq!(accepted["scan_delivery"], "not_runnable");
    let admitted = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload IPC admission");
    assert_eq!(
        admitted
            .pm_control
            .issues
            .get(&42)
            .and_then(|state| state.pinned_next_profile.as_ref()),
        Some(&phase24_launch_profile("codex", "gpt-source")),
        "recover snapshots its exact source profile at request time"
    );
    assert!(admitted
        .pm_control
        .receipts
        .iter()
        .find(|receipt| receipt.operation_id == "recover-replacement-pm")
        .is_some_and(|receipt| receipt.execution_target.is_some()));
    let admitted_bytes = std::fs::read(&prefs_path).expect("admission bytes before exact replay");
    std::fs::remove_file(gwt_core::paths::gwt_sessions_dir().join("session-ecr-42.toml"))
        .expect("remove captured Session after durable admission");
    let (replay_code, replay) = phase24_run_gwtd(
        project.path(),
        home.path(),
        "pm-session-b",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.recover",
            "params": replacement_replay_request,
        }),
    );
    assert!(
        replay_code.success(),
        "same-operation replay recaptured a vanished Session: {replay}"
    );
    assert_eq!(replay["operation_id"], "recover-replacement-pm");
    assert_eq!(
        std::fs::read(&prefs_path).expect("prefs after exact admission replay"),
        admitted_bytes,
        "same-operation replay must consume the durable receipt before inspecting current ECR"
    );

    // Unix exercises the public IPC endpoint: the resident process returns a
    // real but stale atomic projection, while a separate gwtd must compare its
    // control revision with disk and return exactly the offline projection.
    // The server-module companion above remains Windows-safe and pins the same
    // handler/rebase contract without relying on Unix sockets.
    #[cfg(unix)]
    {
        let stale_project = TempDir::new().expect("stale project tempdir");
        phase24_register_pm(stale_project.path(), "pm-status");
        let mut newer = phase24_generated_launch();
        let stale_projection = serde_json::to_value(newer.agent_status())
            .expect("serialize stale resident projection");
        let target = newer
            .pm_control_status(42)
            .target
            .expect("stale fixture target");
        newer
            .admit_pm_control(gwt::IssueMonitorPmControlRequest {
                operation_id: "recover-newer-disk".to_string(),
                project_key: "project-key-3578".to_string(),
                action: gwt::IssueMonitorPmControlAction::Recover,
                actor_session: "pm-status".to_string(),
                actor_registration_generation: 1,
                target,
                reason: "advance disk revision".to_string(),
                requested_profile: Some(phase24_launch_profile("codex", "gpt-source")),
                admitted_at: "2026-08-15T00:20:00Z".to_string(),
                admission_inventory_revision: 50,
            })
            .expect("seed newer disk control revision");
        let stale_prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(stale_project.path());
        gwt::save_issue_monitor_prefs(&stale_prefs_path, &newer.prefs())
            .expect("persist newer disk projection");
        let offline = serde_json::to_value(newer.agent_status()).expect("serialize offline status");
        assert!(
            offline["pm_control"]["revision"]
                .as_u64()
                .zip(stale_projection["pm_control"]["revision"].as_u64())
                .is_some_and(|(disk, resident)| disk > resident),
            "fixture must put disk ahead of the resident endpoint"
        );

        let resident = phase24_spawn_stale_status_daemon(stale_project.path(), stale_projection);
        let (rebased_code, rebased) = phase24_run_gwtd(
            stale_project.path(),
            home.path(),
            "pm-status",
            serde_json::json!({
                "schema_version": 1,
                "operation": "issue.monitor.status",
                "params": { "project_root": stale_project.path() },
            }),
        );
        resident.join().expect("stale status daemon exits");
        assert!(
            rebased_code.success(),
            "rebased public status failed: {rebased}"
        );
        assert_eq!(
            rebased["pm_control"], offline["pm_control"],
            "separate gwtd must return the exact offline target/revision, never stale daemon state"
        );
    }

    // Public `control.reconcile` obtains its observation over the authenticated
    // pane WebSocket. Only the exact, newer, owning-runtime absence proof may
    // cross the PM authority/target revalidation gate. Every alternative is a
    // byte-identical refusal. Replaying the successful operation settles no
    // second prerequisite/effect.
    let unauthorized_project = TempDir::new().expect("unauthorized reconcile project");
    phase24_register_pm(unauthorized_project.path(), "pm-reconcile");
    let unauthorized_target = phase24_seed_pending_reconcile(unauthorized_project.path());
    let unauthorized_issue_path =
        gwt::issue_monitor_prefs_path_for_repo_path(unauthorized_project.path());
    let unauthorized_pm_path =
        gwt::pm_registry::pm_prefs_path_for_repo_path(unauthorized_project.path());
    let issue_before_unauthorized =
        std::fs::read(&unauthorized_issue_path).expect("issue prefs before unauthorized");
    let pm_before_unauthorized =
        std::fs::read(&unauthorized_pm_path).expect("PM prefs before unauthorized");
    let unauthorized_listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind unauthorized no-contact listener");
    unauthorized_listener
        .set_nonblocking(true)
        .expect("make unauthorized listener observable without waiting");
    let unauthorized_ws = format!(
        "ws://{}/internal/pane-ws",
        unauthorized_listener
            .local_addr()
            .expect("unauthorized listener address")
    );
    let _unauthorized_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &unauthorized_ws);
    let _unauthorized_token = ScopedEnvVar::set(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
        "t252-runtime-capability",
    );
    let (unauthorized_code, unauthorized) = phase24_run_gwtd(
        unauthorized_project.path(),
        home.path(),
        "ordinary-session",
        serde_json::json!({
            "schema_version": 1,
            "operation": "issue.monitor.control.reconcile",
            "params": {
                "project_root": unauthorized_project.path(),
                "operation_id": "recover-reconcile-e2e",
                "revoked_generation": unauthorized_target.generation,
            },
        }),
    );
    assert!(!unauthorized_code.success());
    assert_eq!(
        unauthorized,
        serde_json::json!({"status": "refused", "error": "unauthorized"}),
        "authorization runs before replay, runtime contact, or target disclosure"
    );
    assert!(
        matches!(
            unauthorized_listener.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ),
        "an unauthorized caller cannot connect to the owning AppRuntime"
    );
    assert_eq!(
        std::fs::read(&unauthorized_issue_path).expect("issue prefs after unauthorized"),
        issue_before_unauthorized
    );
    assert_eq!(
        std::fs::read(&unauthorized_pm_path).expect("PM prefs after unauthorized"),
        pm_before_unauthorized
    );
    drop(_unauthorized_token);
    drop(_unauthorized_url);

    for (mode, expected_error) in [
        (Phase24InventoryReply::Absent, None),
        (Phase24InventoryReply::Present, Some("still_present")),
        (
            Phase24InventoryReply::Unavailable,
            Some("runtime_unavailable"),
        ),
        (Phase24InventoryReply::Ambiguous, Some("runtime_ambiguous")),
        (Phase24InventoryReply::Stale, Some("inventory_stale")),
        (Phase24InventoryReply::Foreign, Some("target_mismatch")),
    ] {
        let reconcile_project = TempDir::new().expect("reconcile project tempdir");
        if expected_error.is_none() {
            phase24_seed_bound_ecr_generation(reconcile_project.path());
        }
        phase24_register_pm(reconcile_project.path(), "pm-reconcile");
        let target = phase24_seed_pending_reconcile(reconcile_project.path());
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(reconcile_project.path());
        let pm_prefs_path = gwt::pm_registry::pm_prefs_path_for_repo_path(reconcile_project.path());
        let before = std::fs::read(&prefs_path).expect("prefs before public reconcile");
        let pm_before = std::fs::read(&pm_prefs_path).expect("PM prefs before public reconcile");
        let (ws_url, runtime, connections) =
            phase24_spawn_inventory_runtime(mode, reconcile_project.path(), &target, None);
        let _pane_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &ws_url);
        let _pane_token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "t252-runtime-capability",
        );
        let (code, response) = phase24_run_gwtd(
            reconcile_project.path(),
            home.path(),
            "pm-reconcile",
            serde_json::json!({
                "schema_version": 1,
                "operation": "issue.monitor.control.reconcile",
                "params": {
                    "project_root": reconcile_project.path(),
                    "operation_id": "recover-reconcile-e2e",
                    "revoked_generation": target.generation,
                },
            }),
        );
        runtime.join().expect("inventory runtime exits");
        assert_eq!(
            connections.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "authorized reconcile obtains exactly one authenticated runtime proof"
        );
        assert_eq!(
            std::fs::read(&pm_prefs_path).expect("PM prefs after public reconcile"),
            pm_before,
            "runtime proof processing cannot rewrite the durable PM registration"
        );
        drop(_pane_token);
        drop(_pane_url);

        if expected_error.is_none() {
            assert!(code.success(), "exact absence reconcile failed: {response}");
            assert_eq!(response["operation_id"], "recover-reconcile-e2e");
            assert_eq!(response["outcome"], "teardown_settled");
            let settled = gwt::load_issue_monitor_prefs(&prefs_path)
                .expect("reload public reconcile settlement");
            let pending = settled
                .pm_control
                .issues
                .get(&42)
                .and_then(|state| state.pending_control.as_ref())
                .expect("claim cleanup remains pending");
            assert!(pending.teardown_settled);
            assert!(!pending.claim_release_settled);
            assert_eq!(
                settled
                    .pending_effects
                    .iter()
                    .filter(|effect| matches!(
                        &effect.payload,
                        gwt::IssueMonitorEffectPayload::ReleaseControlClaim {
                            operation_id,
                            issue_number: 42,
                            generation: 1,
                            claim_id,
                            owner,
                        } if operation_id == "recover-reconcile-e2e"
                            && claim_id == "claim-42-generation-1"
                            && owner == "host/pm-session-a"
                    ))
                    .count(),
                1,
                "public reconcile prepares one exact durable release"
            );
            let after_first = std::fs::read(&prefs_path).expect("settled bytes");
            let _replay_url = ScopedEnvVar::set(
                gwt_agent::GWT_PANE_WS_URL_ENV,
                "ws://127.0.0.1:9/internal/pane-ws",
            );
            let _replay_token = ScopedEnvVar::set(
                gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
                "t252-runtime-capability",
            );
            let (replay_code, replay) = phase24_run_gwtd(
                reconcile_project.path(),
                home.path(),
                "pm-reconcile",
                serde_json::json!({
                    "schema_version": 1,
                    "operation": "issue.monitor.control.reconcile",
                    "params": {
                        "project_root": reconcile_project.path(),
                        "operation_id": "recover-reconcile-e2e",
                        "revoked_generation": target.generation,
                    },
                }),
            );
            assert!(
                replay_code.success(),
                "settled reconcile replay must not contact runtime: {replay}"
            );
            assert_eq!(
                std::fs::read(&prefs_path).expect("prefs after reconcile replay"),
                after_first,
                "exact absence replay settles exactly once"
            );
        } else {
            assert!(
                !code.success(),
                "non-authoritative proof succeeded: {response}"
            );
            assert_eq!(
                response,
                serde_json::json!({
                    "status": "refused",
                    "error": expected_error.expect("refusal diagnostic"),
                }),
                "reconcile refusal must be stable, typed, and disclose no target identity"
            );
            assert_eq!(
                std::fs::read(&prefs_path).expect("prefs after reconcile refusal"),
                before,
                "present/unavailable/ambiguous/stale/foreign proof must refuse byte-identically"
            );
        }
    }

    // The initial gate must not remain trusted across runtime I/O. Replace the
    // PM after the authenticated query is observed but before its exact absent
    // proof is released; the old caller must fail the nested PM→Issue commit
    // gate without writing either store.
    let race_project = TempDir::new().expect("reconcile replacement race project");
    phase24_register_pm(race_project.path(), "pm-reconcile");
    let race_target = phase24_seed_pending_reconcile(race_project.path());
    let race_issue_path = gwt::issue_monitor_prefs_path_for_repo_path(race_project.path());
    let race_pm_path = gwt::pm_registry::pm_prefs_path_for_repo_path(race_project.path());
    let issue_before_query = std::fs::read(&race_issue_path).expect("issue prefs before query");
    let (query_tx, query_rx) = std::sync::mpsc::channel();
    let (proof_tx, proof_rx) = std::sync::mpsc::channel();
    let (race_ws, race_runtime, race_connections) = phase24_spawn_inventory_runtime(
        Phase24InventoryReply::Absent,
        race_project.path(),
        &race_target,
        Some(Phase24InventoryBarrier {
            query_received: query_tx,
            release_proof: proof_rx,
        }),
    );
    let _race_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &race_ws);
    let _race_token = ScopedEnvVar::set(
        gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
        "t252-runtime-capability",
    );
    let race_project_path = race_project.path().to_path_buf();
    let race_home_path = home.path().to_path_buf();
    let race_child_home = race_home_path.clone();
    let race_child = phase24_spawn_bounded_thread("replacement-race gwtd", move || {
        let _gwt_home = ScopedGwtHome::set(&race_child_home);
        phase24_run_gwtd(
            &race_project_path,
            &race_child_home,
            "pm-reconcile",
            serde_json::json!({
                "schema_version": 1,
                "operation": "issue.monitor.control.reconcile",
                "params": {
                    "project_root": race_project_path.clone(),
                    "operation_id": "recover-reconcile-e2e",
                    "revoked_generation": race_target.generation,
                },
            }),
        )
    });
    query_rx
        .recv_timeout(PHASE24_OPERATION_TIMEOUT)
        .expect("authenticated runtime query reaches the race barrier");
    let replacement_project = race_project.path().to_path_buf();
    let replacement_home = race_home_path.clone();
    phase24_spawn_bounded_thread("durable PM replacement", move || {
        let _gwt_home = ScopedGwtHome::set(&replacement_home);
        assert_eq!(
            gwt_core::paths::gwt_home(),
            replacement_home.join(".gwt"),
            "replacement thread must share the parent's isolated gwt home"
        );
        phase24_register_pm(&replacement_project, "pm-session-b")
    })
    .join()
    .expect("durable PM replacement is bounded");
    let issue_after_replacement =
        std::fs::read(&race_issue_path).expect("issue prefs after PM replacement");
    let pm_after_replacement = std::fs::read(&race_pm_path).expect("PM prefs after replacement");
    assert_eq!(issue_after_replacement, issue_before_query);
    proof_tx.send(()).expect("release exact absent proof");
    race_runtime.join().expect("race runtime exits");
    let (race_code, race_response) = race_child.join().expect("race gwtd exits");
    assert!(!race_code.success());
    assert_eq!(
        race_response,
        serde_json::json!({"status": "refused", "error": "unauthorized"})
    );
    assert_eq!(
        race_connections.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        std::fs::read(&race_issue_path).expect("issue prefs after refused proof"),
        issue_after_replacement,
        "proof acquisition and failed reauthorization cannot commit Issue state"
    );
    assert_eq!(
        std::fs::read(&race_pm_path).expect("PM prefs after refused proof"),
        pm_after_replacement,
        "failed nested authority gate cannot rewrite replacement PM state"
    );
}

/// T-087d: a daemon that accepts and then never answers the handshake must
/// cost a bounded wait and report the outcome as unknown — the PM's loop can
/// live with a slow "I don't know", but not with a hang or a fabricated ack.
#[test]
#[cfg(unix)]
fn hung_daemon_control_publish_is_bounded_and_outcome_unknown() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("project tempdir");
    let home = TempDir::new().expect("home tempdir");
    let _home_guard = ScopedEnvVar::set("HOME", home.path());
    let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());

    let scope = RuntimeScope::from_project_root(project.path(), RuntimeTarget::Host)
        .expect("runtime scope");
    let socket_path = project.path().join("hung.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind hung listener");
    listener
        .set_nonblocking(true)
        .expect("make hung listener nonblocking");
    // Accept connections but never read or write: the client's per-stage
    // deadline is the only thing standing between the PM and a dead loop.
    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let accepting = phase24_spawn_bounded_thread("silent daemon", move || {
        let mut held = Vec::new();
        loop {
            match shutdown_rx.try_recv() {
                Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            match listener.accept() {
                Ok((stream, _)) => held.push(stream),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(PHASE24_POLL_INTERVAL);
                }
                Err(error) => panic!("silent daemon accept failed: {error}"),
            }
        }
    });

    let endpoint = DaemonEndpoint::new(
        scope.clone(),
        std::process::id(),
        socket_path.to_string_lossy().to_string(),
        "test-token".to_string(),
        "test-daemon".to_string(),
    );
    persist_endpoint(
        &scope.endpoint_path(&gwt_core::paths::gwt_home()),
        &endpoint,
    )
    .expect("persist endpoint");

    let started = Instant::now();
    let error = gwt::daemon_publisher::publish_issue_monitor_control(
        project.path(),
        serde_json::json!({ "scan_now": {} }),
    )
    .expect_err("a silent daemon must not produce an ack");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(5),
        "hung daemon must be bounded by the per-stage deadline, waited {elapsed:?}"
    );
    assert!(
        !error.allows_local_fallback(),
        "a live-but-silent daemon still owns authority; local fallback would fork it: {error}"
    );
    shutdown_tx.send(()).expect("stop silent daemon");
    accepting.join().expect("silent daemon exits");
}

/// T-087d: the CLI's durable stop must survive the daemon's own commit path —
/// the real `mutate_issue_monitor_prefs` transaction rebasing the daemon's
/// stale in-memory view — and must not be resurrected by it.
#[test]
fn disk_stop_survives_daemon_commit_and_is_not_resurrected() {
    let _env_lock = env_test_lock();
    let temp = TempDir::new().expect("tempdir");
    let prefs_path = temp.path().join("issue-monitor.json");

    // The daemon's in-memory view: launch still running.
    let mut daemon = launched_monitor();
    gwt::save_issue_monitor_prefs(&prefs_path, &daemon.prefs()).expect("seed prefs");

    // The CLI stop, exactly as run_monitor_stop performs it: a durable
    // transaction on the prefs file.
    let (_, outcome) = gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let mut monitor =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        let outcome = monitor.stop_only(&stop_target(), "provider stuck", NOW);
        if !matches!(outcome, IssueMonitorStopOutcome::Mismatch(_)) {
            *prefs = monitor.prefs();
        }
        Ok(outcome)
    })
    .expect("stop transaction");
    assert!(
        matches!(outcome, IssueMonitorStopOutcome::Stopped { .. }),
        "the live launch must be stoppable: {outcome:?}"
    );

    // The daemon commits its (stale) view through the real transaction path.
    let (committed, _) = gwt::mutate_issue_monitor_prefs(&prefs_path, |disk| {
        daemon.rebase_daemon_driver_prefs(disk);
        *disk = daemon.prefs();
        true
    })
    .expect("daemon commit");

    assert!(
        !daemon.active_issue_numbers().contains(&42),
        "the rebased daemon must adopt the stop instead of resurrecting the launch"
    );
    assert!(
        !committed.priority_order.contains(&42) || daemon.queued_issue_numbers().is_empty(),
        "a stopped launch must not be requeued by the daemon commit"
    );
    let disk_after = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    let daemon_after = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), disk_after);
    assert!(
        !daemon_after.active_issue_numbers().contains(&42),
        "the durable prefs must keep the stop after the daemon commit"
    );

    // A fresh scan over the same candidate must not relaunch the stopped
    // issue behind the PM's back.
    let mut rescanned = daemon.clone();
    scan_issue_monitor_candidates(&mut rescanned, &[issue(42)], "2026-08-09T05:10:00Z");
    assert!(
        !rescanned.queued_issue_numbers().contains(&42),
        "a stop-held issue must stay held across the next scan"
    );
}

/// T-087d: when the durable layer refuses the write (read-only project
/// state), the stop must fail closed with a typed error — and the running
/// launch must remain observably running, because a phantom stop the daemon
/// never saw is unrecoverable for the PM.
#[test]
#[cfg(unix)]
fn unwritable_prefs_fails_the_stop_closed_without_phantom_state() {
    let _env_lock = env_test_lock();
    let temp = TempDir::new().expect("tempdir");
    let state_dir = temp.path().join("state");
    std::fs::create_dir_all(&state_dir).expect("state dir");
    let prefs_path = state_dir.join("issue-monitor.json");

    let monitor = launched_monitor();
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed prefs");

    // Injection: the directory becomes read-only, so the transaction's
    // scratch-and-rename cannot land.
    let mut perms = std::fs::metadata(&state_dir)
        .expect("metadata")
        .permissions();
    let restore = perms.clone();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o555);
    std::fs::set_permissions(&state_dir, perms).expect("set read-only");

    let result = gwt::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let mut inner = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs.clone());
        let outcome = inner.stop_only(&stop_target(), "provider stuck", NOW);
        if !matches!(outcome, IssueMonitorStopOutcome::Mismatch(_)) {
            *prefs = inner.prefs();
        }
        Ok(outcome)
    });
    std::fs::set_permissions(&state_dir, restore).expect("restore permissions");

    assert!(
        result.is_err(),
        "an unpersistable stop must surface a typed error, not a phantom success"
    );
    // The durable contract is the launch accounting (the inbox is rebuilt on
    // scan, not persisted — T-087b): the launch must still be observably
    // running, and the next scan must treat it as running rather than
    // double-queueing or resurrecting it from a half-torn state.
    let reloaded = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    let mut observed = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), reloaded);
    assert!(
        observed.active_issue_numbers().contains(&42),
        "a failed stop must leave the launch observably running"
    );
    scan_issue_monitor_candidates(&mut observed, &[issue(42)], "2026-08-09T05:20:00Z");
    assert!(
        observed.active_issue_numbers().contains(&42),
        "the running launch must survive the next scan untouched"
    );
    assert!(
        !observed.queued_issue_numbers().contains(&42),
        "a still-running launch must not be double-queued after the failed stop"
    );
    assert_eq!(
        observed.inbox_item(42).map(|item| item.state),
        Some(MonitorInboxState::Launched),
        "the rescanned inbox must show the launch as still launched"
    );
}

fn phase24_settle_restart(
    monitor: &mut IssueMonitorState,
    operation_id: &str,
    action: gwt::IssueMonitorPmControlAction,
    requested_profile: gwt::IssueMonitorLaunchProfile,
) -> gwt::IssueMonitorPmControlTarget {
    let target = monitor
        .pm_control_status(42)
        .target
        .expect("generated launch exposes an exact target");
    let request = gwt::IssueMonitorPmControlRequest {
        operation_id: operation_id.to_string(),
        project_key: "project-key-3578".to_string(),
        action,
        actor_session: "pm-session-a".to_string(),
        actor_registration_generation: 1,
        target: target.clone(),
        reason: "provider stalled".to_string(),
        requested_profile: Some(requested_profile),
        admitted_at: "2026-08-15T00:10:00Z".to_string(),
        admission_inventory_revision: 40,
    };
    monitor.admit_pm_control(request).expect("admit restart");
    assert!(
        !monitor.queued_issue_numbers().contains(&42),
        "admission alone must remain teardown-fenced"
    );
    assert_eq!(
        monitor.apply_generation_runtime_event(
            gwt::IssueMonitorGenerationRuntimeEvent::WindowClosed {
                source: target.clone(),
                observed_at: "2026-08-15T00:11:00Z".to_string(),
            },
        ),
        gwt::IssueMonitorGenerationEventDisposition::TeardownSettled
    );
    let settlement = monitor
        .settle_pm_control_claim_release(
            operation_id,
            target.generation,
            target.claim_id.as_deref().expect("claim id"),
            target.claim_owner.as_deref().expect("claim owner"),
            "2026-08-15T00:12:00Z",
        )
        .expect("settle exact release");
    assert!(settlement.should_scan);
    target
}

/// Phase 24 supersedes the historical immediate failover requeue. The durable
/// queue exists only after teardown and exact claim release; once that commit
/// exists, losing the immediate-scan publication must report delivery failure
/// without rolling the queue or receipt back or fabricating success.
#[test]
fn t252_restart_queue_is_durable_when_scan_publication_fails_without_phantom_success() {
    let _env_lock = env_test_lock();
    let project = TempDir::new().expect("project tempdir");
    let home = TempDir::new().expect("home tempdir");
    let _home_guard = ScopedEnvVar::set("HOME", home.path());
    let _userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
    let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project.path());

    let mut monitor = phase24_generated_launch();
    let pinned = phase24_launch_profile("claude", "opus-request-time");
    phase24_settle_restart(
        &mut monitor,
        "failover-scan-publication",
        gwt::IssueMonitorPmControlAction::Failover,
        pinned,
    );
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("seed settled queue");

    // No daemon is registered in this HOME: the immediate-scan publish fails.
    let publish_error = gwt::daemon_publisher::publish_issue_monitor_control(
        project.path(),
        serde_json::json!({ "scan_now": {} }),
    )
    .expect_err("no daemon means no immediate scan");
    assert!(
        publish_error.to_string().contains("daemon"),
        "delivery failure must remain truthful: {publish_error}"
    );

    // The requeue must not depend on that publish: the durable prefs already
    // carry the issue at the priority head for the next scheduled scan.
    let reloaded = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload prefs");
    assert_eq!(
        reloaded.priority_order.first().copied(),
        Some(42),
        "settled restart must keep the issue at the priority head durably"
    );
    let observed = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), reloaded);
    assert!(
        !observed.active_issue_numbers().contains(&42),
        "the revoked launch must not be resurrected by the reload"
    );
    let receipt = observed
        .pm_control_status(42)
        .latest_receipt
        .expect("settled receipt survives failed publication");
    assert_eq!(receipt.operation_id, "failover-scan-publication");
    assert_eq!(receipt.reason, "provider stalled");
    assert_eq!(
        receipt.state,
        gwt::IssueMonitorPmControlReceiptState::Terminal(
            gwt::IssueMonitorPmControlTerminalOutcome::Queued,
        )
    );
}

/// T252 / AS-PM-FAILOVER-FENCE-001: recover pins the source launch profile and
/// failover pins the saved profile selected at request time. In both cases the
/// snapshot, not a later global edit, crosses pending -> queue -> durable
/// delivery exactly once.
#[test]
fn t252_restart_profile_pin_survives_ipc_reload_and_is_consumed_by_one_fresh_delivery() {
    let _env_lock = env_test_lock();
    for (label, action, pinned) in [
        (
            "recover",
            gwt::IssueMonitorPmControlAction::Recover,
            phase24_launch_profile("codex", "gpt-source"),
        ),
        (
            "failover",
            gwt::IssueMonitorPmControlAction::Failover,
            phase24_launch_profile("claude", "opus-request-time"),
        ),
    ] {
        let project = TempDir::new().expect("project tempdir");
        let home = TempDir::new().expect("home tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        assert_eq!(
            gwt_core::paths::gwt_home(),
            home.path().join(".gwt"),
            "profile IPC fixture must remain inside its isolated <home>/.gwt"
        );
        phase24_register_pm(project.path(), "pm-session-a");
        let mut monitor = phase24_generated_launch();
        let attempts_before = monitor.attempt_count(42);
        if action == gwt::IssueMonitorPmControlAction::Failover {
            let mut prefs = monitor.prefs();
            prefs.launch_profile = Some(pinned.clone());
            monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        }
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project.path());
        gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs())
            .expect("persist public control source");
        let (status_code, status) = phase24_run_gwtd(
            project.path(),
            home.path(),
            "pm-session-a",
            serde_json::json!({
                "schema_version": 1,
                "operation": "issue.monitor.status",
                "params": { "project_root": project.path() },
            }),
        );
        assert!(status_code.success(), "{label} status failed: {status}");
        let operation_id = format!("{label}-profile-pin");
        let params = phase24_control_params_from_status(&status, project.path(), &operation_id);
        let (control_code, control) = phase24_run_gwtd(
            project.path(),
            home.path(),
            "pm-session-a",
            serde_json::json!({
                "schema_version": 1,
                "operation": format!("issue.monitor.{label}"),
                "params": params,
            }),
        );
        assert!(control_code.success(), "{label} control failed: {control}");
        assert_eq!(control["operation_id"], operation_id);
        assert_eq!(control["outcome"], "teardown_pending");
        let mut admitted =
            gwt::load_issue_monitor_prefs(&prefs_path).expect("reload gwtd admission");
        assert_eq!(
            admitted
                .pm_control
                .issues
                .get(&42)
                .and_then(|state| state.pinned_next_profile.as_ref()),
            Some(&pinned),
            "{label}: request-time profile is pinned by the real gwtd handler"
        );
        admitted.launch_profile = Some(phase24_launch_profile("codex", "global-edited-later"));
        gwt::save_issue_monitor_prefs(&prefs_path, &admitted)
            .expect("persist global profile edit immediately after admission");
        monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), admitted);
        let admitted_status = monitor.pm_control_status(42);
        assert!(admitted_status.latest_receipt.is_some());
        let target = admitted_status
            .target
            .expect("admitted control retains exact source target");
        assert_eq!(
            monitor.apply_generation_runtime_event(
                gwt::IssueMonitorGenerationRuntimeEvent::WindowClosed {
                    source: target.clone(),
                    observed_at: "2026-08-15T00:11:00Z".to_string(),
                },
            ),
            gwt::IssueMonitorGenerationEventDisposition::TeardownSettled
        );
        monitor
            .settle_pm_control_claim_release(
                &operation_id,
                target.generation,
                target.claim_id.as_deref().expect("claim id"),
                target.claim_owner.as_deref().expect("claim owner"),
                "2026-08-15T00:12:00Z",
            )
            .expect("settle exact release after handler admission");
        gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs())
            .expect("persist settled restart");

        let reloaded = gwt::load_issue_monitor_prefs(&prefs_path).expect("reload queued restart");
        let mut next_daemon =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), reloaded);
        scan_issue_monitor_candidates(&mut next_daemon, &[issue(42)], "2026-08-09T05:00:01Z");
        assert!(next_daemon.apply_confirmed_claim(
            42,
            "claim-42-retry",
            "host/session",
            "effect-42-retry",
            "2026-08-09T05:00:02Z",
        ));
        gwt::save_issue_monitor_prefs(&prefs_path, &next_daemon.prefs())
            .expect("persist fresh delivery");

        let delivery_prefs =
            gwt::load_issue_monitor_prefs(&prefs_path).expect("reload fresh delivery");
        assert_eq!(
            delivery_prefs.pending_launch_deliveries[0].launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::FreshRequired,
            "{label}"
        );
        assert_eq!(
            delivery_prefs.pending_launch_deliveries[0]
                .source_profile
                .as_ref(),
            Some(&pinned),
            "{label}: request-time profile must beat the later global edit"
        );
        assert_eq!(next_daemon.attempt_count(42), attempts_before, "{label}");
        let mut replayed =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), delivery_prefs);
        let first = replayed.take_pending_launch_requests();
        let second = replayed.take_pending_launch_requests();
        assert_eq!(
            first, second,
            "{label}: delivery remains replayable until its ACK"
        );
        assert_eq!(
            first[0].launch_session_strategy,
            IssueMonitorLaunchSessionStrategy::FreshRequired,
            "{label}: reload must not downgrade to ResumeIfSafe"
        );
        assert_eq!(
            first[0].launch_profile.as_ref(),
            Some(&pinned),
            "{label}: the one replayable delivery carries the pinned profile"
        );
        assert!(
            replayed
                .prefs()
                .pm_control
                .issues
                .get(&42)
                .and_then(|state| state.pinned_next_profile.as_ref())
                .is_none(),
            "{label}: queue -> delivery consumes the override exactly once"
        );
    }
}
