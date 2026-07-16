//! Cross-process contract tests for the host-wide index coordinator
//! (SPEC #1939 Phase 70 T-IDX-382 / T-IDX-383, Issue #3264).
//!
//! Real OS processes are spawned by re-executing this test binary with
//! `GWT_COORD_ROLE` set; the `coordinator_helper_entry` test doubles as the
//! helper main. Kernel locks are the exclusion truth, so every assertion
//! observes cross-process behavior (ledger files, markers, kill recovery)
//! instead of in-process state.

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use fs2::FileExt;
use gwt_core::index_coordinator::{
    IndexCoordinator, JobAdmission, JobOutcome, JobPriority, OwnerIdentity, Ticket, TargetKey,
    COORDINATOR_SCHEMA_VERSION,
};

const POLL: Duration = Duration::from_millis(25);

// ---------------------------------------------------------------------------
// Helper-process entry point
// ---------------------------------------------------------------------------

/// Not a real test: when `GWT_COORD_ROLE` is set this executes one helper
/// role inside a spawned copy of this binary and exits. Without the env var
/// it is a no-op so normal test runs pass through.
#[test]
fn coordinator_helper_entry() {
    let Ok(role) = std::env::var("GWT_COORD_ROLE") else {
        return;
    };
    run_helper_role(&role);
}

fn run_helper_role(role: &str) {
    let root = required_env("GWT_COORD_ROOT");
    let coordinator = IndexCoordinator::open(&root).expect("helper: open coordinator");
    match role {
        "exit-now" => {}
        "heavy-job" => {
            let key = target_from_env();
            let hold = Duration::from_millis(required_env_u64("GWT_COORD_HOLD_MS"));
            let ledger = PathBuf::from(required_env("GWT_COORD_LEDGER"));
            let admission = coordinator
                .request_job(&key, JobPriority::Background, Duration::from_secs(20))
                .expect("helper: request job");
            let guard = expect_owner(admission);
            let heavy = guard
                .acquire_heavy(Duration::from_secs(20))
                .expect("helper: acquire heavy");
            locked_counter_add(&ledger, 1);
            std::thread::sleep(hold);
            locked_counter_add(&ledger, -1);
            drop(heavy);
            guard
                .complete(JobOutcome::Completed)
                .expect("helper: complete");
            write_result("done");
        }
        "own-until-waiters" => {
            let key = target_from_env();
            let waiters = required_env_u64("GWT_COORD_WAITERS") as usize;
            let build_count = PathBuf::from(required_env("GWT_COORD_BUILD_COUNT"));
            let started = PathBuf::from(required_env("GWT_COORD_MARKER"));
            let admission = coordinator
                .request_job(&key, JobPriority::Background, Duration::from_secs(20))
                .expect("helper: request job");
            let guard = expect_owner(admission);
            fs::write(&started, b"started").expect("helper: write started marker");
            poll_until(Duration::from_secs(20), || {
                guard.waiter_count().expect("helper: waiter count") >= waiters
            });
            locked_counter_add(&build_count, 1);
            guard
                .complete(JobOutcome::Completed)
                .expect("helper: complete");
            write_result("owner-done");
        }
        "own-until-departure" => {
            let key = target_from_env();
            let build_count = PathBuf::from(required_env("GWT_COORD_BUILD_COUNT"));
            let started = PathBuf::from(required_env("GWT_COORD_MARKER"));
            let saw_two = PathBuf::from(required_env("GWT_COORD_MARKER2"));
            let admission = coordinator
                .request_job(&key, JobPriority::Background, Duration::from_secs(20))
                .expect("helper: request job");
            let guard = expect_owner(admission);
            fs::write(&started, b"started").expect("helper: write started marker");
            poll_until(Duration::from_secs(20), || {
                guard.waiter_count().expect("helper: waiter count") >= 2
            });
            fs::write(&saw_two, b"two-waiters").expect("helper: write waiters marker");
            poll_until(Duration::from_secs(20), || {
                guard.waiter_count().expect("helper: waiter count") <= 1
            });
            locked_counter_add(&build_count, 1);
            guard
                .complete(JobOutcome::Completed)
                .expect("helper: complete");
            write_result("owner-done");
        }
        "join-target" => {
            let key = target_from_env();
            let build_count = PathBuf::from(required_env("GWT_COORD_BUILD_COUNT"));
            let admission = coordinator
                .request_job(&key, JobPriority::Background, Duration::from_secs(20))
                .expect("helper: request job");
            match admission {
                JobAdmission::Owner(guard) => {
                    locked_counter_add(&build_count, 1);
                    guard
                        .complete(JobOutcome::Completed)
                        .expect("helper: complete");
                    write_result("owner-done");
                }
                JobAdmission::Joined(waiter) => {
                    let outcome = waiter
                        .wait(Duration::from_secs(20))
                        .expect("helper: wait for shared outcome");
                    write_result(&format!("waiter:{outcome:?}"));
                }
            }
        }
        "join-then-depart-on-signal" => {
            let key = target_from_env();
            let joined = PathBuf::from(required_env("GWT_COORD_MARKER"));
            let signal = PathBuf::from(required_env("GWT_COORD_SIGNAL"));
            let admission = coordinator
                .request_job(&key, JobPriority::Background, Duration::from_secs(20))
                .expect("helper: request job");
            let waiter = match admission {
                JobAdmission::Joined(waiter) => waiter,
                JobAdmission::Owner(_) => panic!("helper: expected to join, became owner"),
            };
            fs::write(&joined, b"joined").expect("helper: write joined marker");
            poll_until(Duration::from_secs(20), || signal.exists());
            drop(waiter);
            write_result("departed");
        }
        "hold-heavy-and-park" => {
            let key = target_from_env();
            let ready = PathBuf::from(required_env("GWT_COORD_MARKER"));
            let admission = coordinator
                .request_job(&key, JobPriority::Background, Duration::from_secs(20))
                .expect("helper: request job");
            let guard = expect_owner(admission);
            let _heavy = guard
                .acquire_heavy(Duration::from_secs(20))
                .expect("helper: acquire heavy");
            fs::write(&ready, b"ready").expect("helper: write ready marker");
            // Park until the parent kills this process (T-IDX-383 lock owner
            // kill: the kernel must auto-release both locks).
            std::thread::sleep(Duration::from_secs(60));
        }
        other => panic!("unknown helper role: {other}"),
    }
}

fn expect_owner(admission: JobAdmission) -> gwt_core::index_coordinator::TargetJobGuard {
    match admission {
        JobAdmission::Owner(guard) => guard,
        JobAdmission::Joined(_) => panic!("helper: expected job ownership, joined instead"),
    }
}

fn target_from_env() -> TargetKey {
    let raw = required_env("GWT_COORD_TARGET");
    let mut parts = raw.split('|');
    let repo = parts.next().expect("target repo");
    let scope = parts.next().expect("target scope");
    let worktree = parts.next().unwrap_or("");
    if worktree.is_empty() {
        TargetKey::repo_shared(repo, scope)
    } else {
        TargetKey::worktree(repo, scope, worktree)
    }
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("helper env {name} missing"))
}

fn required_env_u64(name: &str) -> u64 {
    required_env(name)
        .parse()
        .unwrap_or_else(|_| panic!("helper env {name} must be u64"))
}

fn write_result(content: &str) {
    let path = PathBuf::from(required_env("GWT_COORD_RESULT"));
    fs::write(path, content).expect("helper: write result");
}

// ---------------------------------------------------------------------------
// Shared cross-process primitives (ledger / polling / spawn)
// ---------------------------------------------------------------------------

/// fs2-locked JSON counter: `{"current": i64, "max": i64}`. Used to observe
/// how many heavy leases are live at once across processes.
fn locked_counter_add(path: &Path, delta: i64) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .expect("open counter");
    file.lock_exclusive().expect("lock counter");
    let mut raw = String::new();
    file.read_to_string(&mut raw).expect("read counter");
    let (mut current, mut max) = parse_counter(&raw);
    current += delta;
    if current > max {
        max = current;
    }
    file.seek(SeekFrom::Start(0)).expect("seek counter");
    file.set_len(0).expect("truncate counter");
    file.write_all(format!("{{\"current\":{current},\"max\":{max}}}").as_bytes())
        .expect("write counter");
    fs2::FileExt::unlock(&file).expect("unlock counter");
}

fn read_counter(path: &Path) -> (i64, i64) {
    let raw = fs::read_to_string(path).unwrap_or_default();
    parse_counter(&raw)
}

fn parse_counter(raw: &str) -> (i64, i64) {
    if raw.trim().is_empty() {
        return (0, 0);
    }
    let value: serde_json::Value = serde_json::from_str(raw).expect("counter json");
    (
        value["current"].as_i64().unwrap_or(0),
        value["max"].as_i64().unwrap_or(0),
    )
}

fn poll_until(deadline: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if done() {
            return;
        }
        std::thread::sleep(POLL);
    }
    panic!("poll_until timed out after {deadline:?}");
}

fn wait_for_file(path: &Path, deadline: Duration) {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if path.exists() {
            return;
        }
        std::thread::sleep(POLL);
    }
    panic!("file {} did not appear within {deadline:?}", path.display());
}

struct HelperSpawn {
    child: Child,
    label: String,
}

fn spawn_helper(label: &str, envs: &[(&str, String)]) -> HelperSpawn {
    let exe = std::env::current_exe().expect("current test binary");
    let mut command = Command::new(exe);
    command
        .arg("coordinator_helper_entry")
        .arg("--exact")
        .arg("--nocapture")
        .arg("--test-threads=1");
    for (key, value) in envs {
        command.env(key, value);
    }
    let child = command.spawn().expect("spawn helper process");
    HelperSpawn {
        child,
        label: label.to_string(),
    }
}

fn wait_success(mut spawn: HelperSpawn, deadline: Duration) {
    let start = Instant::now();
    loop {
        match spawn.child.try_wait().expect("try_wait helper") {
            Some(status) => {
                assert!(
                    status.success(),
                    "helper {} exited with {status}",
                    spawn.label
                );
                return;
            }
            None if start.elapsed() >= deadline => {
                let _ = spawn.child.kill();
                panic!("helper {} did not exit within {deadline:?}", spawn.label);
            }
            None => std::thread::sleep(POLL),
        }
    }
}

struct TestArena {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    coord_root: PathBuf,
}

impl TestArena {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let coord_root = root.join("coordinator");
        Self {
            _tmp: tmp,
            root,
            coord_root,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn coord_env(&self) -> (&str, String) {
        (
            "GWT_COORD_ROOT",
            self.coord_root.to_string_lossy().into_owned(),
        )
    }
}

fn write_stale_ticket(path: &Path, target: &TargetKey, pid: u32, start_id: &str) {
    let ticket = Ticket {
        schema_version: COORDINATOR_SCHEMA_VERSION,
        target: target.file_stem(),
        priority: JobPriority::Background,
        owner: OwnerIdentity {
            pid,
            start_id: start_id.to_string(),
        },
        acquired_at_ms: 0,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create ticket dir");
    }
    fs::write(path, serde_json::to_vec(&ticket).expect("ticket json")).expect("write ticket");
}

// ---------------------------------------------------------------------------
// T-IDX-382: host-wide exclusion / coalesce / queue / waiter departure
// ---------------------------------------------------------------------------

#[test]
fn heavy_lease_is_host_wide_exclusive_across_processes() {
    let arena = TestArena::new();
    let ledger = arena.path("ledger.json");
    // Distinct targets across two repos and two worktrees: each becomes its
    // own job owner, but the heavy lease must still serialize host-wide
    // (FR-379: at most one model-loaded runner tree).
    let jobs = [
        ("job-a", "repo-a|files|wt-1"),
        ("job-b", "repo-a|issues|"),
        ("job-c", "repo-b|files|wt-2"),
    ];
    let mut children = Vec::new();
    for (label, target) in jobs {
        let result = arena.path(&format!("result-{label}"));
        children.push(spawn_helper(
            label,
            &[
                ("GWT_COORD_ROLE", "heavy-job".to_string()),
                arena.coord_env(),
                ("GWT_COORD_TARGET", target.to_string()),
                ("GWT_COORD_HOLD_MS", "250".to_string()),
                (
                    "GWT_COORD_LEDGER",
                    ledger.to_string_lossy().into_owned(),
                ),
                (
                    "GWT_COORD_RESULT",
                    result.to_string_lossy().into_owned(),
                ),
            ],
        ));
    }
    for child in children {
        wait_success(child, Duration::from_secs(60));
    }
    for (label, _) in jobs {
        let result = arena.path(&format!("result-{label}"));
        assert_eq!(
            fs::read_to_string(&result).expect("read result"),
            "done",
            "job {label} must finish through the queued heavy lease"
        );
    }
    let (current, max) = read_counter(&ledger);
    assert_eq!(current, 0, "all heavy leases must be released");
    assert_eq!(
        max, 1,
        "heavy lease must never be held by more than one process host-wide"
    );
}

#[test]
fn same_target_requests_coalesce_into_single_shared_build() {
    let arena = TestArena::new();
    let build_count = arena.path("build-count.json");
    let started = arena.path("owner-started");
    let target = "repo-a|files|wt-1";

    let owner = spawn_helper(
        "owner",
        &[
            ("GWT_COORD_ROLE", "own-until-waiters".to_string()),
            arena.coord_env(),
            ("GWT_COORD_TARGET", target.to_string()),
            ("GWT_COORD_WAITERS", "2".to_string()),
            (
                "GWT_COORD_BUILD_COUNT",
                build_count.to_string_lossy().into_owned(),
            ),
            (
                "GWT_COORD_MARKER",
                started.to_string_lossy().into_owned(),
            ),
            (
                "GWT_COORD_RESULT",
                arena.path("result-owner").to_string_lossy().into_owned(),
            ),
        ],
    );
    wait_for_file(&started, Duration::from_secs(20));

    let joiners: Vec<HelperSpawn> = ["join-1", "join-2"]
        .into_iter()
        .map(|label| {
            spawn_helper(
                label,
                &[
                    ("GWT_COORD_ROLE", "join-target".to_string()),
                    arena.coord_env(),
                    ("GWT_COORD_TARGET", target.to_string()),
                    (
                        "GWT_COORD_BUILD_COUNT",
                        build_count.to_string_lossy().into_owned(),
                    ),
                    (
                        "GWT_COORD_RESULT",
                        arena
                            .path(&format!("result-{label}"))
                            .to_string_lossy()
                            .into_owned(),
                    ),
                ],
            )
        })
        .collect();

    wait_success(owner, Duration::from_secs(60));
    for joiner in joiners {
        wait_success(joiner, Duration::from_secs(60));
    }

    let (count, _) = read_counter(&build_count);
    assert_eq!(
        count, 1,
        "same-target concurrent requests must coalesce into one shared build"
    );
    for label in ["join-1", "join-2"] {
        let result = fs::read_to_string(arena.path(&format!("result-{label}")))
            .expect("read joiner result");
        assert!(
            result.contains("Completed"),
            "joiner {label} must receive the shared completed outcome, got {result}"
        );
    }
}

#[test]
fn waiter_departure_keeps_shared_job_running() {
    let arena = TestArena::new();
    let build_count = arena.path("build-count.json");
    let started = arena.path("owner-started");
    let saw_two = arena.path("owner-saw-two");
    let joined = arena.path("departing-joined");
    let depart_signal = arena.path("depart-now");
    let target = "repo-a|files|wt-1";

    let owner = spawn_helper(
        "owner",
        &[
            ("GWT_COORD_ROLE", "own-until-departure".to_string()),
            arena.coord_env(),
            ("GWT_COORD_TARGET", target.to_string()),
            (
                "GWT_COORD_BUILD_COUNT",
                build_count.to_string_lossy().into_owned(),
            ),
            (
                "GWT_COORD_MARKER",
                started.to_string_lossy().into_owned(),
            ),
            (
                "GWT_COORD_MARKER2",
                saw_two.to_string_lossy().into_owned(),
            ),
            (
                "GWT_COORD_RESULT",
                arena.path("result-owner").to_string_lossy().into_owned(),
            ),
        ],
    );
    wait_for_file(&started, Duration::from_secs(20));

    let departing = spawn_helper(
        "departing",
        &[
            (
                "GWT_COORD_ROLE",
                "join-then-depart-on-signal".to_string(),
            ),
            arena.coord_env(),
            ("GWT_COORD_TARGET", target.to_string()),
            (
                "GWT_COORD_MARKER",
                joined.to_string_lossy().into_owned(),
            ),
            (
                "GWT_COORD_SIGNAL",
                depart_signal.to_string_lossy().into_owned(),
            ),
            (
                "GWT_COORD_RESULT",
                arena
                    .path("result-departing")
                    .to_string_lossy()
                    .into_owned(),
            ),
        ],
    );
    let staying = spawn_helper(
        "staying",
        &[
            ("GWT_COORD_ROLE", "join-target".to_string()),
            arena.coord_env(),
            ("GWT_COORD_TARGET", target.to_string()),
            (
                "GWT_COORD_BUILD_COUNT",
                build_count.to_string_lossy().into_owned(),
            ),
            (
                "GWT_COORD_RESULT",
                arena.path("result-staying").to_string_lossy().into_owned(),
            ),
        ],
    );

    // Owner sees both waiters, then one departs; the shared build must keep
    // running for the remaining waiter (AS-8).
    wait_for_file(&saw_two, Duration::from_secs(20));
    fs::write(&depart_signal, b"go").expect("write depart signal");
    wait_success(departing, Duration::from_secs(60));
    wait_success(owner, Duration::from_secs(60));
    wait_success(staying, Duration::from_secs(60));

    let (count, _) = read_counter(&build_count);
    assert_eq!(count, 1, "the shared build must run exactly once");
    let staying_result =
        fs::read_to_string(arena.path("result-staying")).expect("read staying result");
    assert!(
        staying_result.contains("Completed"),
        "remaining waiter must still receive the shared outcome, got {staying_result}"
    );
}

// ---------------------------------------------------------------------------
// T-IDX-383: fault injection (owner kill / stale ticket / crash before spawn
// / PID reuse)
// ---------------------------------------------------------------------------

#[test]
fn killed_heavy_owner_releases_locks_for_next_claimant() {
    let arena = TestArena::new();
    let ready = arena.path("owner-ready");
    let target = TargetKey::worktree("repo-a", "files", "wt-1");

    let mut parked = spawn_helper(
        "parked-owner",
        &[
            ("GWT_COORD_ROLE", "hold-heavy-and-park".to_string()),
            arena.coord_env(),
            ("GWT_COORD_TARGET", "repo-a|files|wt-1".to_string()),
            (
                "GWT_COORD_MARKER",
                ready.to_string_lossy().into_owned(),
            ),
        ],
    );
    wait_for_file(&ready, Duration::from_secs(20));
    parked.child.kill().expect("kill parked owner");
    let _ = parked.child.wait();

    // Kernel locks must auto-release with the dead owner; the next claimant
    // recovers without manual cleanup (T-IDX-383).
    let coordinator = IndexCoordinator::open(&arena.coord_root).expect("open coordinator");
    let admission = coordinator
        .request_job(&target, JobPriority::Background, Duration::from_secs(10))
        .expect("request job after owner kill");
    let guard = match admission {
        JobAdmission::Owner(guard) => guard,
        JobAdmission::Joined(_) => panic!("dead owner must not hold the target job"),
    };
    let heavy = guard
        .acquire_heavy(Duration::from_secs(10))
        .expect("heavy lease after owner kill");
    drop(heavy);
    guard
        .complete(JobOutcome::Completed)
        .expect("complete recovered job");
}

#[test]
fn stale_ticket_without_lock_does_not_block_claimant() {
    let arena = TestArena::new();
    let target = TargetKey::worktree("repo-a", "files", "wt-1");
    let coordinator = IndexCoordinator::open(&arena.coord_root).expect("open coordinator");

    // Obtain a real-but-dead PID (crash before spawn / stale metadata).
    let dead = spawn_helper(
        "dead-pid",
        &[
            ("GWT_COORD_ROLE", "exit-now".to_string()),
            arena.coord_env(),
        ],
    );
    let dead_pid = dead.child.id();
    wait_success(dead, Duration::from_secs(30));

    write_stale_ticket(
        &coordinator.target_ticket_path(&target),
        &target,
        dead_pid,
        "stale-start-id",
    );
    write_stale_ticket(
        &coordinator.heavy_ticket_path(),
        &target,
        dead_pid,
        "stale-start-id",
    );

    let admission = coordinator
        .request_job(&target, JobPriority::Background, Duration::from_secs(2))
        .expect("stale ticket must not block admission");
    let guard = match admission {
        JobAdmission::Owner(guard) => guard,
        JobAdmission::Joined(_) => panic!("stale ticket must not look like a live owner"),
    };
    let heavy = guard
        .acquire_heavy(Duration::from_secs(2))
        .expect("stale heavy ticket must not block the lease");
    drop(heavy);
    guard
        .complete(JobOutcome::Completed)
        .expect("complete after stale-ticket recovery");
}

#[test]
fn pid_reuse_ticket_with_live_pid_is_treated_as_stale() {
    let arena = TestArena::new();
    let target = TargetKey::repo_shared("repo-a", "issues");
    let coordinator = IndexCoordinator::open(&arena.coord_root).expect("open coordinator");

    // PID reuse equivalent: the ticket names this very-much-alive process,
    // but with a different process start identity. Since no kernel lock is
    // held, the claimant must proceed (kernel lock is the only truth).
    write_stale_ticket(
        &coordinator.target_ticket_path(&target),
        &target,
        std::process::id(),
        "some-other-process-start",
    );
    write_stale_ticket(
        &coordinator.heavy_ticket_path(),
        &target,
        std::process::id(),
        "some-other-process-start",
    );

    let admission = coordinator
        .request_job(&target, JobPriority::Background, Duration::from_secs(2))
        .expect("pid-reuse ticket must not block admission");
    let guard = match admission {
        JobAdmission::Owner(guard) => guard,
        JobAdmission::Joined(_) => panic!("pid-reuse ticket must be treated as stale"),
    };
    let heavy = guard
        .acquire_heavy(Duration::from_secs(2))
        .expect("pid-reuse heavy ticket must not block the lease");
    drop(heavy);
    guard
        .complete(JobOutcome::Completed)
        .expect("complete after pid-reuse recovery");
}
