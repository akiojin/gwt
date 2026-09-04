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
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fs2::FileExt;
use gwt_core::index::broker::{
    RefreshBroker, RefreshBrokerClock, RefreshIntent, RefreshReason, RefreshResourceClass,
    RefreshScope, RefreshTarget, RefreshTargetState, REFRESH_INTENT_PROTOCOL_VERSION,
};
use gwt_core::index_coordinator::{
    IndexCoordinator, JobAdmission, JobOutcome, JobPriority, LeaseEventKind, OwnerIdentity,
    TargetKey, Ticket, COORDINATOR_SCHEMA_VERSION,
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
    if role == "claim-refresh-broker" {
        let ready = PathBuf::from(required_env("GWT_COORD_MARKER"));
        let start_signal = PathBuf::from(required_env("GWT_COORD_SIGNAL"));
        let attempted = PathBuf::from(required_env("GWT_COORD_MARKER2"));
        let complete_signal = PathBuf::from(required_env("GWT_COORD_SIGNAL2"));
        let broker =
            RefreshBroker::open(&root, Duration::ZERO).expect("helper: open refresh broker");
        fs::write(&ready, b"ready").expect("helper: write refresh broker ready marker");
        poll_until(Duration::from_secs(20), || start_signal.exists());
        let claim = broker.claim_next().expect("helper: claim refresh target");
        write_result(if claim.is_some() { "claimed" } else { "idle" });
        fs::write(&attempted, b"attempted").expect("helper: write claim attempted marker");
        poll_until(Duration::from_secs(20), || complete_signal.exists());
        if let Some(claim) = claim {
            claim.complete().expect("helper: complete refresh target");
        }
        return;
    }

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
        "verification-job" => {
            // SPEC #3576 T-001: a heavy verification run is an ordinary
            // claimant of the same host-wide heavy lease, so it must
            // serialize against index jobs on the shared ledger.
            let key = verification_target_from_env();
            let hold = Duration::from_millis(required_env_u64("GWT_COORD_HOLD_MS"));
            let ttl = Duration::from_millis(required_env_u64("GWT_COORD_TTL_MS"));
            let ledger = PathBuf::from(required_env("GWT_COORD_LEDGER"));
            let admission = coordinator
                .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(20))
                .expect("helper: request verification job");
            let guard = expect_owner(admission);
            let lease = guard
                .acquire_heavy_with_ttl(Duration::from_secs(20), ttl)
                .expect("helper: acquire verification lease");
            locked_counter_add(&ledger, 1);
            std::thread::sleep(hold);
            locked_counter_add(&ledger, -1);
            lease.release().expect("helper: release verification lease");
            guard
                .complete(JobOutcome::Completed)
                .expect("helper: complete");
            write_result("done");
        }
        "hold-verification-and-park" => {
            // SPEC #3576 T-006: the owner parks while holding the lease so
            // the parent can kill it and prove kernel auto-release still
            // wins over a not-yet-expired TTL.
            let key = verification_target_from_env();
            let ttl = Duration::from_millis(required_env_u64("GWT_COORD_TTL_MS"));
            let ready = PathBuf::from(required_env("GWT_COORD_MARKER"));
            let admission = coordinator
                .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(20))
                .expect("helper: request verification job");
            let guard = expect_owner(admission);
            let _lease = guard
                .acquire_heavy_with_ttl(Duration::from_secs(20), ttl)
                .expect("helper: acquire verification lease");
            fs::write(&ready, b"ready").expect("helper: write ready marker");
            std::thread::sleep(Duration::from_secs(60));
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

/// `GWT_COORD_VERIFY_TARGET` is `repo_hash|worktree_hash`; the scope is fixed
/// by [`TargetKey::verification`].
fn verification_target_from_env() -> TargetKey {
    let raw = required_env("GWT_COORD_VERIFY_TARGET");
    let mut parts = raw.split('|');
    let repo = parts.next().expect("verification repo");
    let worktree = parts.next().expect("verification worktree");
    TargetKey::verification(repo, worktree)
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
    let mut command = gwt_core::process::hidden_command(exe);
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
        lease_id: None,
        expires_at_ms: None,
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
                ("GWT_COORD_LEDGER", ledger.to_string_lossy().into_owned()),
                ("GWT_COORD_RESULT", result.to_string_lossy().into_owned()),
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
            ("GWT_COORD_MARKER", started.to_string_lossy().into_owned()),
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
        let result =
            fs::read_to_string(arena.path(&format!("result-{label}"))).expect("read joiner result");
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
            ("GWT_COORD_MARKER", started.to_string_lossy().into_owned()),
            ("GWT_COORD_MARKER2", saw_two.to_string_lossy().into_owned()),
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
            ("GWT_COORD_ROLE", "join-then-depart-on-signal".to_string()),
            arena.coord_env(),
            ("GWT_COORD_TARGET", target.to_string()),
            ("GWT_COORD_MARKER", joined.to_string_lossy().into_owned()),
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
// SPEC #3576 T-001 / T-003 / T-006: verification as a heavy-lease claimant
// ---------------------------------------------------------------------------

#[test]
fn verification_serializes_against_index_jobs_host_wide() {
    let arena = TestArena::new();
    let ledger = arena.path("ledger.json");
    // Two index jobs across different repos plus two verification runs from
    // different worktrees. Every one of them owns a distinct target job, so
    // only the host-wide heavy lease can keep them from overlapping (FR-1:
    // the contended resource is host CPU, not the repository).
    let mut children = Vec::new();
    for (label, target) in [
        ("index-a", "repo-a|files|wt-1"),
        ("index-b", "repo-b|issues|"),
    ] {
        children.push(spawn_helper(
            label,
            &[
                ("GWT_COORD_ROLE", "heavy-job".to_string()),
                arena.coord_env(),
                ("GWT_COORD_TARGET", target.to_string()),
                ("GWT_COORD_HOLD_MS", "250".to_string()),
                ("GWT_COORD_LEDGER", ledger.to_string_lossy().into_owned()),
                (
                    "GWT_COORD_RESULT",
                    arena
                        .path(&format!("result-{label}"))
                        .to_string_lossy()
                        .into_owned(),
                ),
            ],
        ));
    }
    for (label, target) in [("verify-a", "repo-a|wt-1"), ("verify-b", "repo-b|wt-9")] {
        children.push(spawn_helper(
            label,
            &[
                ("GWT_COORD_ROLE", "verification-job".to_string()),
                arena.coord_env(),
                ("GWT_COORD_VERIFY_TARGET", target.to_string()),
                ("GWT_COORD_HOLD_MS", "250".to_string()),
                ("GWT_COORD_TTL_MS", "120000".to_string()),
                ("GWT_COORD_LEDGER", ledger.to_string_lossy().into_owned()),
                (
                    "GWT_COORD_RESULT",
                    arena
                        .path(&format!("result-{label}"))
                        .to_string_lossy()
                        .into_owned(),
                ),
            ],
        ));
    }
    for child in children {
        wait_success(child, Duration::from_secs(90));
    }
    for label in ["index-a", "index-b", "verify-a", "verify-b"] {
        assert_eq!(
            fs::read_to_string(arena.path(&format!("result-{label}"))).expect("read result"),
            "done",
            "claimant {label} must finish through the queued heavy lease"
        );
    }
    let (current, max) = read_counter(&ledger);
    assert_eq!(current, 0, "all heavy leases must be released");
    assert_eq!(
        max, 1,
        "verification and index jobs must never hold the heavy lease at the same time"
    );

    // FR-5: acquire / release are recorded per lease id.
    let coordinator = IndexCoordinator::open(&arena.coord_root).expect("open coordinator");
    let events = coordinator.lease_events().expect("read lease events");
    let released: Vec<_> = events
        .iter()
        .filter(|event| event.kind == LeaseEventKind::Released)
        .collect();
    assert_eq!(
        released.len(),
        2,
        "both verification leases must record an explicit release: {events:?}"
    );
    assert!(
        events
            .iter()
            .filter(|event| event.kind == LeaseEventKind::Acquired)
            .count()
            >= 2,
        "verification acquisitions must be recorded: {events:?}"
    );
}

#[test]
fn verification_lease_holds_its_target_job_lock() {
    let arena = TestArena::new();
    let ready = arena.path("verify-ready");
    let key = TargetKey::verification("repo-a", "wt-1");

    let mut parked = spawn_helper(
        "verify-holder",
        &[
            ("GWT_COORD_ROLE", "hold-verification-and-park".to_string()),
            arena.coord_env(),
            ("GWT_COORD_VERIFY_TARGET", "repo-a|wt-1".to_string()),
            ("GWT_COORD_TTL_MS", "120000".to_string()),
            ("GWT_COORD_MARKER", ready.to_string_lossy().into_owned()),
        ],
    );
    wait_for_file(&ready, Duration::from_secs(30));

    // Lock order target job -> heavy (FR-392 / FR-2b): holding the heavy
    // lease implies the verification target job is still owned, so a
    // same-key claimant joins instead of taking a second target slot.
    let coordinator = IndexCoordinator::open(&arena.coord_root).expect("open coordinator");
    match coordinator
        .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(5))
        .expect("request verification job")
    {
        JobAdmission::Joined(waiter) => drop(waiter),
        JobAdmission::Owner(_) => {
            let _ = parked.child.kill();
            panic!("verification lease must keep its target job lock held");
        }
    }

    let status = coordinator
        .heavy_lease_status()
        .expect("read heavy lease status");
    assert!(status.held, "the verification lease must read as held");
    assert_eq!(
        status.target.as_deref(),
        Some(key.file_stem().as_str()),
        "status must name the verification claimant"
    );
    assert!(
        !status.expired,
        "a lease inside its TTL must not read as expired"
    );
    assert!(
        status.remaining_ms.is_some_and(|remaining| remaining > 0),
        "status must report the remaining TTL, got {:?}",
        status.remaining_ms
    );

    parked.child.kill().expect("kill verification holder");
    let _ = parked.child.wait();
}

#[test]
fn killed_verification_owner_releases_lease_before_ttl_expiry() {
    let arena = TestArena::new();
    let ready = arena.path("verify-ready");
    let key = TargetKey::verification("repo-a", "wt-1");

    let mut parked = spawn_helper(
        "verify-holder",
        &[
            ("GWT_COORD_ROLE", "hold-verification-and-park".to_string()),
            arena.coord_env(),
            ("GWT_COORD_VERIFY_TARGET", "repo-a|wt-1".to_string()),
            // A TTL far beyond the test: only the kernel auto-release can
            // hand the lease over (the pre-existing T-IDX-383 guarantee must
            // survive the TTL addition).
            ("GWT_COORD_TTL_MS", "3600000".to_string()),
            ("GWT_COORD_MARKER", ready.to_string_lossy().into_owned()),
        ],
    );
    wait_for_file(&ready, Duration::from_secs(30));
    parked.child.kill().expect("kill verification holder");
    let _ = parked.child.wait();

    let coordinator = IndexCoordinator::open(&arena.coord_root).expect("open coordinator");
    let guard = match coordinator
        .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(10))
        .expect("request verification job after kill")
    {
        JobAdmission::Owner(guard) => guard,
        JobAdmission::Joined(_) => panic!("dead verification owner must not hold the target job"),
    };
    let lease = guard
        .acquire_heavy_with_ttl(Duration::from_secs(10), Duration::from_secs(60))
        .expect("verification lease after owner kill");
    lease.release().expect("release recovered lease");
    guard
        .complete(JobOutcome::Completed)
        .expect("complete recovered job");
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
            ("GWT_COORD_MARKER", ready.to_string_lossy().into_owned()),
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

// ---------------------------------------------------------------------------
// SPEC #1939 Phase 71 T-IDX-434: deterministic Refresh Broker contract
// ---------------------------------------------------------------------------

const REFRESH_QUIET_PERIOD: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct ManualRefreshBrokerClock {
    now_millis: AtomicU64,
}

impl ManualRefreshBrokerClock {
    fn new(now_millis: u64) -> Self {
        Self {
            now_millis: AtomicU64::new(now_millis),
        }
    }

    fn advance(&self, duration: Duration) {
        let millis = u64::try_from(duration.as_millis()).expect("test duration fits u64 millis");
        self.now_millis.fetch_add(millis, Ordering::SeqCst);
    }
}

impl RefreshBrokerClock for ManualRefreshBrokerClock {
    fn now_millis(&self) -> u64 {
        self.now_millis.load(Ordering::SeqCst)
    }
}

fn open_refresh_broker(arena: &TestArena) -> (RefreshBroker, Arc<ManualRefreshBrokerClock>) {
    let clock = Arc::new(ManualRefreshBrokerClock::new(10_000));
    let broker = RefreshBroker::open_with_clock(
        arena.path("refresh-broker"),
        REFRESH_QUIET_PERIOD,
        clock.clone(),
    )
    .expect("open refresh broker");
    (broker, clock)
}

fn file_scopes() -> [RefreshScope; 2] {
    [RefreshScope::Files, RefreshScope::FilesDocs]
}

fn base_refresh_target(repo_hash: impl Into<String>) -> RefreshTarget {
    RefreshTarget::base(repo_hash, file_scopes())
}

fn overlay_refresh_target(
    repo_hash: impl Into<String>,
    worktree_hash: impl Into<String>,
) -> RefreshTarget {
    RefreshTarget::overlay(repo_hash, worktree_hash, file_scopes())
}

fn refresh_intent(
    target: RefreshTarget,
    desired_epoch: u64,
    desired_snapshot: impl Into<String>,
    priority: JobPriority,
    reason: RefreshReason,
) -> RefreshIntent {
    RefreshIntent {
        protocol_version: REFRESH_INTENT_PROTOCOL_VERSION,
        target,
        desired_epoch,
        desired_snapshot: desired_snapshot.into(),
        priority,
        reason,
        resource_class: RefreshResourceClass::Embedding,
    }
}

fn dirty_refresh_intent(target: RefreshTarget, desired_epoch: u64) -> RefreshIntent {
    refresh_intent(
        target,
        desired_epoch,
        format!("snapshot-{desired_epoch}"),
        JobPriority::Background,
        RefreshReason::DirtyEvent,
    )
}

fn recursive_file_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, directory: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .expect("read broker state directory")
            .map(|entry| entry.expect("read broker state entry"))
            .collect();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type().expect("read broker entry type");
            if file_type.is_dir() {
                visit(root, &path, files);
            } else if file_type.is_file() {
                files.push((
                    path.strip_prefix(root)
                        .expect("broker entry below root")
                        .to_path_buf(),
                    fs::read(&path).expect("read broker state bytes"),
                ));
            }
        }
    }

    let mut files = Vec::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn refresh_broker_coalesces_one_thousand_quiet_events_to_latest_snapshot_and_deadline() {
    let arena = TestArena::new();
    let (broker, clock) = open_refresh_broker(&arena);
    let target = overlay_refresh_target("repo-a", "wt-1");

    for epoch in 1..=1_000 {
        broker
            .submit(dirty_refresh_intent(target.clone(), epoch))
            .expect("submit dirty refresh intent");
        if epoch < 1_000 {
            clock.advance(Duration::from_millis(1));
        }
    }
    let last_event_at = clock.now_millis();
    let stale_delay = Duration::from_secs(10);
    clock.advance(stale_delay);
    broker
        .submit(dirty_refresh_intent(target.clone(), 7))
        .expect("submit delayed stale epoch");

    let snapshot = broker.inspect().expect("inspect quiet broker");
    assert_eq!(snapshot.target_count(), 1, "one target state must remain");
    assert!(
        snapshot.queue_depth() <= 1,
        "events are not queue entries: {}",
        snapshot.queue_depth()
    );
    assert_eq!(snapshot.running_count(), 0, "quiet events start no owner");
    assert_eq!(snapshot.targets().len(), 1);
    let pending = snapshot.target(&target).expect("coalesced target snapshot");
    assert_eq!(pending.target(), &target);
    assert_eq!(pending.desired_epoch(), 1_000);
    assert_eq!(pending.desired_snapshot(), "snapshot-1000");
    assert_eq!(pending.priority(), JobPriority::Background);
    assert_eq!(pending.state(), RefreshTargetState::Quiet);
    assert_eq!(
        pending.quiet_deadline_millis(),
        Some(last_event_at + 30_000),
        "quiet deadline is measured from the latest epoch and is not reset by a delayed old event"
    );

    clock.advance(REFRESH_QUIET_PERIOD - stale_delay - Duration::from_millis(1));
    assert!(
        broker
            .claim_next()
            .expect("claim before deadline")
            .is_none(),
        "the one thousand events must start no work before the quiet deadline"
    );
    clock.advance(Duration::from_millis(1));
    let claim = broker
        .claim_next()
        .expect("claim at deadline")
        .expect("latest coalesced intent becomes claimable at its deadline");
    assert_eq!(
        claim.intent().protocol_version,
        REFRESH_INTENT_PROTOCOL_VERSION
    );
    assert_eq!(claim.intent().target, target);
    assert_eq!(claim.intent().desired_epoch, 1_000);
    assert_eq!(claim.intent().desired_snapshot, "snapshot-1000");
    assert_eq!(claim.intent().reason, RefreshReason::DirtyEvent);
    assert_eq!(
        claim.intent().resource_class,
        RefreshResourceClass::Embedding
    );
    claim.complete().expect("complete coalesced refresh");
}

#[test]
fn refresh_broker_resets_quiet_deadline_from_the_last_dirty_event() {
    let arena = TestArena::new();
    let (broker, clock) = open_refresh_broker(&arena);
    let target = base_refresh_target("repo-a");

    broker
        .submit(dirty_refresh_intent(target.clone(), 1))
        .expect("submit initial dirty intent");
    clock.advance(Duration::from_secs(29));
    broker
        .submit(dirty_refresh_intent(target.clone(), 2))
        .expect("submit resetting dirty intent");
    let reset_at = clock.now_millis();

    let snapshot = broker.inspect().expect("inspect reset deadline");
    let pending = snapshot.target(&target).expect("base target snapshot");
    assert_eq!(pending.desired_epoch(), 2);
    assert_eq!(pending.desired_snapshot(), "snapshot-2");
    assert_eq!(
        pending.quiet_deadline_millis(),
        Some(reset_at + 30_000),
        "every later dirty event resets the full quiet period"
    );

    clock.advance(Duration::from_secs(1));
    assert!(
        broker
            .claim_next()
            .expect("claim at old deadline")
            .is_none(),
        "the first event's deadline must no longer admit work"
    );
    clock.advance(Duration::from_secs(29));
    let claim = broker
        .claim_next()
        .expect("claim at reset deadline")
        .expect("target becomes ready thirty seconds after the last event");
    assert_eq!(claim.intent().desired_epoch, 2);
    claim.complete().expect("complete reset refresh");
}

#[test]
fn refresh_broker_running_storm_keeps_one_latest_follow_up_and_no_second_owner() {
    let arena = TestArena::new();
    let (broker, clock) = open_refresh_broker(&arena);
    let target = overlay_refresh_target("repo-a", "wt-1");

    broker
        .submit(dirty_refresh_intent(target.clone(), 1))
        .expect("submit first refresh");
    clock.advance(REFRESH_QUIET_PERIOD);
    let first = broker
        .claim_next()
        .expect("claim first refresh")
        .expect("first refresh ready");

    for epoch in 2..=1_001 {
        broker
            .submit(dirty_refresh_intent(target.clone(), epoch))
            .expect("submit dirty intent during run");
    }

    let snapshot = broker.inspect().expect("inspect running storm");
    assert_eq!(snapshot.target_count(), 1);
    assert_eq!(snapshot.running_count(), 1);
    let running = snapshot.target(&target).expect("running target snapshot");
    assert_eq!(running.state(), RefreshTargetState::DirtyDuringRun);
    assert_eq!(running.follow_up_count(), 1, "follow-up state is bounded");
    assert_eq!(running.desired_epoch(), 1_001);
    assert_eq!(running.desired_snapshot(), "snapshot-1001");
    assert!(
        broker
            .claim_next()
            .expect("same-target claim while running")
            .is_none(),
        "a running target must never gain a concurrent second owner"
    );

    first.complete().expect("complete first refresh");
    let after_complete = broker.inspect().expect("inspect queued follow-up");
    let follow_up = after_complete
        .target(&target)
        .expect("follow-up target snapshot");
    assert_eq!(follow_up.state(), RefreshTargetState::Quiet);
    assert_eq!(follow_up.follow_up_count(), 1);

    clock.advance(REFRESH_QUIET_PERIOD);
    let second = broker
        .claim_next()
        .expect("claim coalesced follow-up")
        .expect("one follow-up becomes ready");
    assert_eq!(second.intent().desired_epoch, 1_001);
    assert_eq!(second.intent().desired_snapshot, "snapshot-1001");
    second.complete().expect("complete follow-up refresh");
    assert!(
        broker
            .claim_next()
            .expect("claim after follow-up")
            .is_none(),
        "the storm must not leave event-sized follow-up jobs"
    );
    let ready = broker.inspect().expect("inspect completed follow-up");
    let ready = ready.target(&target).expect("completed target snapshot");
    assert_eq!(ready.state(), RefreshTargetState::Ready);
    assert_eq!(ready.follow_up_count(), 0);
}

#[test]
fn refresh_broker_queue_depth_is_bounded_by_distinct_normalized_targets() {
    let arena = TestArena::new();
    let (broker, _clock) = open_refresh_broker(&arena);
    const DISTINCT_TARGETS: usize = 40;
    const EVENTS_PER_TARGET: usize = 25;

    for event in 0..EVENTS_PER_TARGET {
        for target_index in 0..DISTINCT_TARGETS {
            let scopes = if event % 2 == 0 {
                [RefreshScope::Files, RefreshScope::FilesDocs]
            } else {
                [RefreshScope::FilesDocs, RefreshScope::Files]
            };
            let target = RefreshTarget::overlay("repo-bound", format!("wt-{target_index}"), scopes);
            let epoch = u64::try_from(event * DISTINCT_TARGETS + target_index + 1)
                .expect("test epoch fits u64");
            broker
                .submit(dirty_refresh_intent(target, epoch))
                .expect("submit bounded queue intent");
        }
    }
    for target_index in 0..DISTINCT_TARGETS {
        let target = RefreshTarget::overlay(
            "repo-bound",
            format!("wt-{target_index}"),
            [RefreshScope::FilesDocs, RefreshScope::Files],
        );
        broker
            .submit(refresh_intent(
                target,
                2_000 + u64::try_from(target_index).expect("target index fits u64"),
                format!("manual-{target_index}"),
                JobPriority::ManualRebuild,
                RefreshReason::Manual,
            ))
            .expect("promote distinct target into the runnable queue");
    }

    let snapshot = broker.inspect().expect("inspect bounded queue");
    assert_eq!(
        snapshot.target_count(),
        DISTINCT_TARGETS,
        "scope-set order must normalize to the same logical target"
    );
    assert_eq!(
        snapshot.queue_depth(),
        DISTINCT_TARGETS,
        "promoted targets occupy exactly one queue entry per distinct normalized target"
    );
    assert_eq!(snapshot.targets().len(), DISTINCT_TARGETS);
    assert_eq!(snapshot.running_count(), 0);
}

#[test]
fn refresh_broker_two_processes_cannot_claim_the_same_target_concurrently() {
    let arena = TestArena::new();
    let broker_root = arena.path("refresh-broker-concurrent");
    let broker = RefreshBroker::open(&broker_root, Duration::ZERO)
        .expect("open refresh broker for concurrent processes");
    let target = overlay_refresh_target("repo-concurrent", "wt-1");
    broker
        .submit(dirty_refresh_intent(target, 1))
        .expect("submit concurrent claim target");

    let start_signal = arena.path("refresh-claim-start-signal");
    let complete_signal = arena.path("refresh-claim-complete-signal");
    let mut helpers = Vec::new();
    let mut results = Vec::new();
    for index in 0..2 {
        let ready = arena.path(&format!("refresh-claim-ready-{index}"));
        let attempted = arena.path(&format!("refresh-claim-attempted-{index}"));
        let result = arena.path(&format!("refresh-claim-result-{index}"));
        helpers.push(spawn_helper(
            &format!("refresh-claimant-{index}"),
            &[
                ("GWT_COORD_ROLE", "claim-refresh-broker".to_string()),
                ("GWT_COORD_ROOT", broker_root.to_string_lossy().into_owned()),
                ("GWT_COORD_MARKER", ready.to_string_lossy().into_owned()),
                (
                    "GWT_COORD_SIGNAL",
                    start_signal.to_string_lossy().into_owned(),
                ),
                (
                    "GWT_COORD_MARKER2",
                    attempted.to_string_lossy().into_owned(),
                ),
                (
                    "GWT_COORD_SIGNAL2",
                    complete_signal.to_string_lossy().into_owned(),
                ),
                ("GWT_COORD_RESULT", result.to_string_lossy().into_owned()),
            ],
        ));
        results.push((ready, attempted, result));
    }
    for (ready, _, _) in &results {
        wait_for_file(ready, Duration::from_secs(20));
    }
    fs::write(&start_signal, b"claim").expect("release refresh claimants");
    for (_, attempted, _) in &results {
        wait_for_file(attempted, Duration::from_secs(20));
    }

    let claimed = results
        .iter()
        .filter(|(_, _, result)| fs::read_to_string(result).is_ok_and(|value| value == "claimed"))
        .count();
    assert_eq!(
        claimed, 1,
        "two OS processes must expose exactly one same-target owner while both claims are live"
    );
    fs::write(&complete_signal, b"complete").expect("release refresh claim owner");
    for helper in helpers {
        wait_success(helper, Duration::from_secs(20));
    }
}

#[test]
fn refresh_broker_promotes_priority_while_inspect_remains_read_only() {
    let arena = TestArena::new();
    let (broker, _clock) = open_refresh_broker(&arena);
    let broker_root = arena.path("refresh-broker");
    let promoted_target = base_refresh_target("repo-priority-a");
    let quiet_target = base_refresh_target("repo-priority-b");

    broker
        .submit(dirty_refresh_intent(promoted_target.clone(), 1))
        .expect("submit promotable background intent");
    broker
        .submit(dirty_refresh_intent(quiet_target.clone(), 1))
        .expect("submit second background intent");
    broker
        .submit(refresh_intent(
            promoted_target.clone(),
            1,
            "snapshot-1",
            JobPriority::InteractiveSearch,
            RefreshReason::Search,
        ))
        .expect("promote target through interactive intent");
    broker
        .submit(dirty_refresh_intent(promoted_target.clone(), 2))
        .expect("submit newer background epoch after promotion");

    let bytes_before_inspect = recursive_file_bytes(&broker_root);
    for _ in 0..3 {
        let snapshot = broker.inspect().expect("read broker status");
        assert_eq!(snapshot.target_count(), 2);
        assert!(
            snapshot.queue_depth() <= snapshot.target_count(),
            "queue depth must remain bounded by distinct targets"
        );
        assert_eq!(snapshot.running_count(), 0);
        let promoted = snapshot
            .target(&promoted_target)
            .expect("promoted target snapshot");
        assert_eq!(promoted.target(), &promoted_target);
        assert_eq!(promoted.priority(), JobPriority::InteractiveSearch);
        assert_eq!(promoted.state(), RefreshTargetState::Queued);
        assert_eq!(promoted.desired_epoch(), 2);
        assert_eq!(promoted.desired_snapshot(), "snapshot-2");
        assert_eq!(promoted.follow_up_count(), 0);
        assert_eq!(
            snapshot
                .target(&quiet_target)
                .expect("quiet target snapshot")
                .state(),
            RefreshTargetState::Quiet
        );
    }
    assert_eq!(
        recursive_file_bytes(&broker_root),
        bytes_before_inspect,
        "inspect must not change, append, or create durable broker bytes"
    );

    let claim = broker
        .claim_next()
        .expect("claim after read-only inspections")
        .expect("inspection must not consume promoted work");
    assert_eq!(claim.intent().target, promoted_target);
    assert_eq!(claim.intent().priority, JobPriority::InteractiveSearch);
    assert_eq!(claim.intent().desired_epoch, 2);
    assert_eq!(claim.intent().desired_snapshot, "snapshot-2");
    claim.complete().expect("complete promoted refresh");

    assert!(
        broker
            .claim_next()
            .expect("claim quiet background target")
            .is_none(),
        "inspect and promotion must not bypass another target's quiet period"
    );
}
