//! Issue #3633: production must own a subject that starts the runtime daemon.
//!
//! Before this suite the only caller that ever reached `serve_blocking` was a
//! human typing `gwtd daemon start`. Every production caller that resolved
//! `DaemonBootstrapAction::Spawn` declined to spawn, so `daemon.subscribe`
//! and the Issue Monitor control lane were permanently unavailable while the
//! monitor still reported a healthy snapshot.

#![cfg(unix)]

use std::{
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use gwt::daemon_supervisor::{
    daemon_spawn_request, daemon_stderr_log_path, DaemonEnsureInputs, DaemonEnsureOutcome,
    DaemonSpawnContext, DaemonSupervisor,
};
use gwt_core::daemon::{
    persist_endpoint, DaemonEndpoint, RuntimeScope, RuntimeTarget, DAEMON_PROTOCOL_VERSION,
};
use gwt_core::process::{resolved_command, ProcessPlanRequest};
use tempfile::TempDir;

fn spawn_stand_in(program: &str, args: &[&str]) -> std::io::Result<Child> {
    resolved_command(ProcessPlanRequest::new(program).args(args))
        .map_err(|error| std::io::Error::other(error.to_string()))?
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

/// A stand-in daemon: a child that stays alive until it is killed, so the
/// supervisor's "one child per scope" bookkeeping can be exercised without
/// building or binding a real IPC server.
fn spawn_long_lived_child(_context: &DaemonSpawnContext<'_>) -> std::io::Result<Child> {
    spawn_stand_in("sleep", &["30"])
}

fn spawn_immediately_exiting_child(_context: &DaemonSpawnContext<'_>) -> std::io::Result<Child> {
    spawn_stand_in("true", &[])
}

fn counting_supervisor(
    calls: Arc<AtomicUsize>,
    inner: fn(&DaemonSpawnContext<'_>) -> std::io::Result<Child>,
) -> DaemonSupervisor {
    DaemonSupervisor::with_spawner(move |context| {
        calls.fetch_add(1, Ordering::SeqCst);
        inner(context)
    })
}

fn always_alive(_pid: u32) -> bool {
    true
}

fn seed_live_endpoint(gwt_home: &Path, project_root: &Path, pid: u32) -> RuntimeScope {
    let scope = RuntimeScope::from_project_root(project_root, RuntimeTarget::Host).expect("scope");
    let endpoint = DaemonEndpoint::new(
        scope.clone(),
        pid,
        gwt_home.join("daemon.sock").display().to_string(),
        "auth-token".to_string(),
        "9.99.0".to_string(),
    );
    persist_endpoint(&scope.endpoint_path(gwt_home), &endpoint).expect("persist endpoint");
    scope
}

fn wait_until(deadline: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    predicate()
}

struct Fixture {
    _temp: TempDir,
    gwt_home: PathBuf,
    project_root: PathBuf,
}

fn fixture() -> Fixture {
    let temp = TempDir::new().expect("tempdir");
    let gwt_home = temp.path().join("gwt-home");
    let project_root = temp.path().join("project");
    std::fs::create_dir_all(&gwt_home).expect("gwt home");
    std::fs::create_dir_all(&project_root).expect("project root");
    Fixture {
        _temp: temp,
        gwt_home,
        project_root,
    }
}

/// AC-1: an empty endpoint slot is the signal that nobody is serving the
/// runtime daemon. Production has to answer it by starting one.
#[test]
fn ensure_running_starts_a_daemon_when_no_endpoint_is_published() {
    let fixture = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let supervisor = counting_supervisor(Arc::clone(&calls), spawn_long_lived_child);

    let outcome = supervisor
        .ensure_running_with(
            &fixture.project_root,
            DaemonEnsureInputs {
                gwt_home: fixture.gwt_home.clone(),
                is_process_alive: &always_alive,
            },
        )
        .expect("ensure running");

    assert!(
        matches!(outcome, DaemonEnsureOutcome::Spawned { pid } if pid > 0),
        "an empty endpoint slot must start a daemon, got {outcome:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    supervisor.shutdown();
}

/// A daemon that already owns the endpoint must never be duplicated: two
/// drivers on one project is the Issue Monitor duplicate-launch trap.
#[test]
fn ensure_running_reuses_a_live_daemon_endpoint_instead_of_spawning() {
    let fixture = fixture();
    seed_live_endpoint(&fixture.gwt_home, &fixture.project_root, std::process::id());
    let calls = Arc::new(AtomicUsize::new(0));
    let supervisor = counting_supervisor(Arc::clone(&calls), spawn_long_lived_child);

    let outcome = supervisor
        .ensure_running_with(
            &fixture.project_root,
            DaemonEnsureInputs {
                gwt_home: fixture.gwt_home.clone(),
                is_process_alive: &always_alive,
            },
        )
        .expect("ensure running");

    assert_eq!(
        outcome,
        DaemonEnsureOutcome::AlreadyRunning {
            pid: std::process::id()
        }
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "no second daemon may start"
    );
    supervisor.shutdown();
}

/// A daemon needs time between `fork` and publishing its endpoint. Without
/// in-flight tracking every tick during that window starts another daemon.
#[test]
fn ensure_running_does_not_start_a_second_daemon_while_the_first_is_still_starting() {
    let fixture = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let supervisor = counting_supervisor(Arc::clone(&calls), spawn_long_lived_child);
    let inputs = || DaemonEnsureInputs {
        gwt_home: fixture.gwt_home.clone(),
        is_process_alive: &always_alive,
    };

    let first = supervisor
        .ensure_running_with(&fixture.project_root, inputs())
        .expect("first ensure");
    let second = supervisor
        .ensure_running_with(&fixture.project_root, inputs())
        .expect("second ensure");

    let DaemonEnsureOutcome::Spawned { pid } = first else {
        panic!("first ensure must spawn, got {first:?}");
    };
    assert_eq!(second, DaemonEnsureOutcome::Starting { pid });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    supervisor.shutdown();
}

/// AC-2: a daemon that crashed or exited must be replaced without operator
/// action. The supervisor recognises its own dead child and starts a new one.
#[test]
fn ensure_running_replaces_a_daemon_that_exited() {
    let fixture = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let supervisor = counting_supervisor(Arc::clone(&calls), spawn_immediately_exiting_child);
    let inputs = || DaemonEnsureInputs {
        gwt_home: fixture.gwt_home.clone(),
        is_process_alive: &always_alive,
    };

    let first = supervisor
        .ensure_running_with(&fixture.project_root, inputs())
        .expect("first ensure");
    assert!(matches!(first, DaemonEnsureOutcome::Spawned { .. }));

    assert!(
        wait_until(Duration::from_secs(5), || {
            !supervisor.has_live_child_for(&fixture.project_root, &fixture.gwt_home)
        }),
        "the supervisor must reap its exited child instead of leaving a zombie"
    );

    let second = supervisor
        .ensure_running_with(&fixture.project_root, inputs())
        .expect("second ensure");
    assert!(
        matches!(second, DaemonEnsureOutcome::Spawned { .. }),
        "an exited daemon must be replaced, got {second:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    supervisor.shutdown();
}

/// A stale endpoint left by a dead daemon must not be mistaken for a live one.
#[test]
fn ensure_running_starts_a_daemon_when_the_published_endpoint_owner_is_dead() {
    let fixture = fixture();
    seed_live_endpoint(&fixture.gwt_home, &fixture.project_root, 424_242);
    let calls = Arc::new(AtomicUsize::new(0));
    let supervisor = counting_supervisor(Arc::clone(&calls), spawn_long_lived_child);
    let never_alive = |_pid: u32| false;

    let outcome = supervisor
        .ensure_running_with(
            &fixture.project_root,
            DaemonEnsureInputs {
                gwt_home: fixture.gwt_home.clone(),
                is_process_alive: &never_alive,
            },
        )
        .expect("ensure running");

    assert!(matches!(outcome, DaemonEnsureOutcome::Spawned { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    supervisor.shutdown();
}

/// A spawn failure must surface the reason instead of being swallowed: a
/// silently missing daemon is exactly the failure mode this Issue is about.
#[test]
fn ensure_running_reports_the_reason_when_the_daemon_cannot_be_started() {
    let fixture = fixture();
    let supervisor = DaemonSupervisor::with_spawner(|_context: &DaemonSpawnContext<'_>| {
        Err(std::io::Error::other("gwtd binary could not be resolved"))
    });

    let error = supervisor
        .ensure_running_with(
            &fixture.project_root,
            DaemonEnsureInputs {
                gwt_home: fixture.gwt_home.clone(),
                is_process_alive: &always_alive,
            },
        )
        .expect_err("spawn failure must be reported");

    assert!(
        error.contains("gwtd binary could not be resolved"),
        "the failure reason must reach the caller: {error}"
    );
    supervisor.shutdown();
}

/// The child must ask for the daemon through gwtd's only sanctioned transport
/// (a stdin JSON envelope — legacy argv is refused with exit 2), and it must
/// run inside the project it serves because the daemon derives its
/// `RuntimeScope` from its own cwd.
#[test]
fn daemon_spawn_request_asks_gwtd_for_daemon_start_inside_the_project() {
    let gwtd = PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd");
    let project_root = PathBuf::from("/Users/example/Workbench/project");

    let request = daemon_spawn_request(&gwtd, &project_root);

    assert_eq!(request.program, gwtd);
    assert_eq!(request.current_dir, project_root);
    let envelope: serde_json::Value =
        serde_json::from_str(request.stdin_envelope.trim()).expect("stdin envelope is JSON");
    assert_eq!(envelope["schema_version"], 1);
    assert_eq!(envelope["operation"], "daemon.start");
    assert!(
        envelope["params"].is_object(),
        "gwtd rejects an envelope whose params is not an object: {envelope}"
    );
    assert!(
        request.stdin_envelope.ends_with('\n'),
        "gwtd reads one newline-delimited envelope line"
    );
}

/// The endpoint protocol version the supervisor resolves against must be the
/// one the daemon publishes, or every ensure would spawn a duplicate.
#[test]
fn supervisor_resolves_against_the_current_daemon_protocol_version() {
    let fixture = fixture();
    let scope = seed_live_endpoint(&fixture.gwt_home, &fixture.project_root, std::process::id());
    let endpoint_path = scope.endpoint_path(&fixture.gwt_home);
    let mut endpoint: DaemonEndpoint =
        serde_json::from_slice(&std::fs::read(&endpoint_path).expect("read endpoint"))
            .expect("parse endpoint");
    assert_eq!(endpoint.protocol_version, DAEMON_PROTOCOL_VERSION);

    endpoint.protocol_version = DAEMON_PROTOCOL_VERSION + 1;
    persist_endpoint(&endpoint_path, &endpoint).expect("persist mismatched endpoint");

    let calls = Arc::new(AtomicUsize::new(0));
    let supervisor = counting_supervisor(Arc::clone(&calls), spawn_long_lived_child);

    let outcome = supervisor
        .ensure_running_with(
            &fixture.project_root,
            DaemonEnsureInputs {
                gwt_home: fixture.gwt_home.clone(),
                is_process_alive: &always_alive,
            },
        )
        .expect("ensure running");

    assert!(
        matches!(outcome, DaemonEnsureOutcome::Spawned { .. }),
        "an endpoint from another protocol version cannot be reused: {outcome:?}"
    );
    supervisor.shutdown();
}

/// The child's diagnostics have to land somewhere. The first isolated
/// end-to-end run of this supervisor produced a daemon that exited instantly
/// with its real reason (`path must be shorter than SUN_LEN`, Issue #3476)
/// discarded to `/dev/null` — a silent daemon failure, which is the same class
/// of bug Issue #3633 exists to remove.
#[test]
fn the_spawn_context_anchors_the_daemon_diagnostic_log_next_to_its_endpoint() {
    let fixture = fixture();
    let seen: Arc<std::sync::Mutex<Option<PathBuf>>> = Arc::new(std::sync::Mutex::new(None));
    let recorder = Arc::clone(&seen);
    let supervisor = DaemonSupervisor::with_spawner(move |context: &DaemonSpawnContext<'_>| {
        *recorder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(context.endpoint_path.to_path_buf());
        spawn_stand_in("sleep", &["30"])
    });

    supervisor
        .ensure_running_with(
            &fixture.project_root,
            DaemonEnsureInputs {
                gwt_home: fixture.gwt_home.clone(),
                is_process_alive: &always_alive,
            },
        )
        .expect("ensure running");

    let endpoint_path = seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .expect("the spawner receives the endpoint it is expected to publish");
    let scope =
        RuntimeScope::from_project_root(&fixture.project_root, RuntimeTarget::Host).expect("scope");
    assert_eq!(endpoint_path, scope.endpoint_path(&fixture.gwt_home));

    let stderr_log = daemon_stderr_log_path(&endpoint_path);
    assert_eq!(
        stderr_log.parent(),
        endpoint_path.parent(),
        "the diagnostic log belongs beside the endpoint slot an operator inspects"
    );
    assert_ne!(stderr_log, endpoint_path, "it must not shadow the endpoint");
    assert!(stderr_log.to_string_lossy().ends_with(".daemon-stderr.log"));
    supervisor.shutdown();
}
