//! SPEC #3576: canonical verification admission across real `gwtd` invocations.
//!
//! Canonical runs serialize in the same or different worktrees, release their
//! locks after success or failure, and leave ordinary development tests alone.
//! Self-exec command fixtures use readiness/release handshakes so assertions
//! observe active commands rather than relying on a guessed sleep duration.

#![cfg(unix)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use gwt_core::process::hidden_command;
use tempfile::TempDir;

const SESSION: &str = "session-admission-test";

fn spawn_gwtd(home: &Path, cwd: &Path, envelope: &str, extra_env: &[(&str, &Path)]) -> Child {
    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
    for key in [
        "GWT_BIN_PATH",
        "GWT_BROWSER_URL_FILE",
        "GWT_HOOK_BIN",
        "GWT_PROJECT_ROOT",
        "GWT_REPO_HASH",
        "GWT_SESSION_KIND",
        "GWT_SESSION_RUNTIME_PATH",
        "GWT_WORKTREE_HASH",
    ] {
        command.env_remove(key);
    }
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GWT_SESSION_ID", SESSION)
        // Issue #4409: this suite is about lease admission and serialization,
        // not about where verification is hosted. `verify.run` refuses to
        // spawn in place when its launcher runs at a degraded nice value and
        // no daemon can take the work, so without this declaration the suite
        // would pass in CI and in a terminal and fail inside an agent — an
        // outcome decided by where the test runner was started, not by the
        // behaviour under test.
        .env("GWT_VERIFY_SPAWN_HOST", "inherit")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gwtd");
    child
        .stdin
        .take()
        .expect("gwtd stdin")
        .write_all(envelope.as_bytes())
        .expect("write envelope");
    child
}

fn gwtd(home: &Path, cwd: &Path, envelope: &str) -> (bool, String) {
    collect_gwtd(spawn_gwtd(home, cwd, envelope, &[]))
}

fn collect_gwtd(child: Child) -> (bool, String) {
    let output = child.wait_with_output().expect("await gwtd");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let payload = serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .map(|envelope| {
            ["output", "error"]
                .iter()
                .filter_map(|key| envelope.get(key).and_then(|value| value.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|payload| !payload.is_empty())
        .unwrap_or(stdout);
    (output.status.success(), format!("{payload}{stderr}"))
}

fn git(cwd: &Path, args: &[&str]) {
    let status = hidden_command("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|err| panic!("spawn git {args:?}: {err}"));
    assert!(status.success(), "git {args:?} failed in {}", cwd.display());
}

struct Arena {
    home: TempDir,
    /// Owns the repository and its sibling worktree until the test ends.
    _root: TempDir,
    repo: PathBuf,
    sibling: PathBuf,
}

impl Arena {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("home tempdir");
        let root = tempfile::tempdir().expect("repo root tempdir");
        let repo = root.path().join("main");
        let sibling = root.path().join("sibling");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["commit", "--allow-empty", "-qm", "init"]);
        // A second worktree of the same repository is the shape of the
        // production incident (test-only: agents never create worktrees).
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                sibling.to_str().unwrap(),
                "-b",
                "sibling",
            ],
        );
        Self {
            home,
            _root: root,
            repo,
            sibling,
        }
    }

    fn run_in(&self, cwd: &Path, envelope: &str) -> (bool, String) {
        gwtd(self.home.path(), cwd, envelope)
    }

    /// Spawn a process that looks like a test binary of the sibling worktree.
    fn spawn_sibling_heavy(&self) -> Child {
        let ready = self.home.path().join("development-ready");
        let mut child = hidden_command(std::env::current_exe().expect("test binary path"))
            .args(["--ignored", "--exact", "fake_heavy_process_parks"])
            .env("ADMISSION_DEVELOPMENT_READY", &ready)
            .current_dir(&self.sibling)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn fake heavy process");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready.exists() {
            if child.try_wait().unwrap().is_some() || Instant::now() >= deadline {
                kill(child);
                panic!("development test command did not become ready");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        child
    }
}

fn verify_command(command: &str, max_wait_secs: u64) -> String {
    serde_json::json!({
        "schema_version": 1,
        "operation": "verify.run",
        "params": {"commands": [command], "max_wait_secs": max_wait_secs}
    })
    .to_string()
}

fn verify_run(max_wait_secs: u64) -> String {
    verify_command("git --version", max_wait_secs)
}

const STATUS: &str = r#"{"schema_version":1,"operation":"verify.lease.status","params":{}}"#;

fn kill(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// A real canonical run whose command remains active until the test releases it.
struct CanonicalRun {
    child: Option<Child>,
    release: PathBuf,
}

impl CanonicalRun {
    fn start(arena: &Arena, cwd: &Path) -> Self {
        let ready = arena.home.path().join("canonical-ready");
        let release = arena.home.path().join("canonical-release");
        let exe = std::env::current_exe().unwrap();
        let command = format!(
            "\"{}\" --ignored --exact canonical_command_parks",
            exe.display()
        );
        let child = spawn_gwtd(
            arena.home.path(),
            cwd,
            &verify_command(&command, 0),
            &[("ADMISSION_READY", &ready), ("ADMISSION_RELEASE", &release)],
        );
        let mut run = Self {
            child: Some(child),
            release,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready.exists() {
            if run.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                let (_, output) = collect_gwtd(run.child.take().unwrap());
                panic!("canonical run exited before its command started: {output}");
            }
            assert!(Instant::now() < deadline, "canonical command did not start");
            std::thread::sleep(Duration::from_millis(10));
        }
        run
    }

    fn finish(mut self) -> (bool, String) {
        std::fs::write(&self.release, "release").unwrap();
        collect_gwtd(self.child.take().unwrap())
    }
}

impl Drop for CanonicalRun {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.release, "release");
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}

#[test]
#[ignore = "spawned as the canonical verification command"]
fn canonical_command_parks() {
    let ready = std::env::var_os("ADMISSION_READY").unwrap();
    let release = PathBuf::from(std::env::var_os("ADMISSION_RELEASE").unwrap());
    std::fs::write(ready, "ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !release.exists() {
        assert!(
            Instant::now() < deadline,
            "canonical test command was not released"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "spawned as the sibling worktree's development test binary"]
fn fake_heavy_process_parks() {
    std::fs::write(
        std::env::var_os("ADMISSION_DEVELOPMENT_READY").unwrap(),
        "ready",
    )
    .unwrap();
    std::thread::sleep(Duration::from_secs(120));
}

#[test]
fn verify_run_does_not_wait_for_sibling_development_tests() {
    let arena = Arena::new();
    let mut heavy = arena.spawn_sibling_heavy();
    let (ok, output) = arena.run_in(&arena.repo, &verify_run(0));
    let still_running = heavy.try_wait().unwrap().is_none();
    let (_, status) = arena.run_in(&arena.repo, STATUS);
    kill(heavy);

    assert!(
        still_running,
        "the ordinary development test must remain running"
    );
    assert!(
        ok,
        "ordinary development work must not defer canonical verification:\n{output}"
    );
    assert!(output.contains("verify: PASS"), "{output}");
    assert!(status.starts_with("verification lease: free"), "{status}");
}

fn assert_canonical_runs_are_serialized(same_worktree: bool) {
    let arena = Arena::new();
    let first = CanonicalRun::start(&arena, &arena.repo);
    let contender = if same_worktree {
        &arena.repo
    } else {
        &arena.sibling
    };
    let (ok, output) = arena.run_in(contender, &verify_run(0));
    assert!(!ok, "a concurrent canonical run must defer:\n{output}");
    assert!(output.contains("deferred"), "{output}");
    assert!(
        !output.contains("verify: PASS") && !output.contains("verify: FAIL"),
        "{output}"
    );
    let (_, status) = arena.run_in(contender, STATUS);
    assert!(status.starts_with("verification lease: held"), "{status}");
    let (ok, output) = first.finish();
    assert!(ok && output.contains("verify: PASS"), "{output}");
    let (ok, output) = arena.run_in(contender, &verify_run(0));
    assert!(ok && output.contains("verify: PASS"), "{output}");
    let (_, status) = arena.run_in(contender, STATUS);
    assert!(status.starts_with("verification lease: free"), "{status}");
}

#[test]
fn canonical_runs_in_the_same_worktree_are_serialized() {
    assert_canonical_runs_are_serialized(true);
}

#[test]
fn canonical_runs_in_different_worktrees_are_serialized() {
    assert_canonical_runs_are_serialized(false);
}

#[test]
fn failed_canonical_commands_release_the_lease() {
    for command in ["sh -c 'exit 7'", "gwt-missing-verification-command-3576"] {
        let arena = Arena::new();
        let (ok, output) = arena.run_in(&arena.repo, &verify_command(command, 0));
        assert!(!ok && output.contains("verify: FAIL"), "{output}");
        let (_, status) = arena.run_in(&arena.repo, STATUS);
        assert!(status.starts_with("verification lease: free"), "{status}");
        let (ok, output) = arena.run_in(&arena.sibling, &verify_run(0));
        assert!(ok && output.contains("verify: PASS"), "{output}");
    }
}

// Issue #4789: kill the canonical runner itself, not its command. The command's
// ready marker proves the run started, and the record transition is the event
// we await; no elapsed duration decides the ordering.
fn assert_killed_runner_is_recorded(signal: i32) {
    let arena = Arena::new();
    let (ok, output) = arena.run_in(&arena.repo, &verify_run(0));
    assert!(ok, "initial successful run: {output}");
    let record_path = arena.repo.join(".gwt/skill-state/verification-run.json");
    let previous: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    let mut run = CanonicalRun::start(&arena, &arena.repo);
    let running: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    let mut child = run.child.take().unwrap();
    // SAFETY: this PID belongs to the child we just spawned and have not reaped.
    assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
    let status = child.wait().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(signal));
    std::fs::write(&run.release, "release").unwrap();

    assert_ne!(
        running["record_id"], previous["record_id"],
        "a new run must replace old PASS before commands start"
    );
    assert_eq!(running["all_passed"], false);
    assert_eq!(running["lifecycle"]["status"], "running");
    let mirror_path = arena.repo.join(".gwt/tmp/verify-run.json");
    let deadline = Instant::now() + Duration::from_secs(30);
    let interrupted = loop {
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&mirror_path).unwrap()).unwrap();
        if record["lifecycle"]["status"] == "interrupted" {
            break record;
        }
        assert!(
            Instant::now() < deadline,
            "runner termination was not recorded: {record}"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(interrupted["record_id"], running["record_id"]);
    assert_eq!(interrupted["all_passed"], false);
    assert!(interrupted["lifecycle"]["reason"]
        .as_str()
        .unwrap()
        .contains("external"));
    assert!(interrupted["lifecycle"]["current_command"]
        .as_str()
        .unwrap()
        .contains("canonical_command_parks"));
    let mirror: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(
        mirror, interrupted,
        "the diagnostic mirror must carry the same nonempty record"
    );
    let record = serde_json::from_value(interrupted).unwrap();
    let evidence = gwt::cli::verification_record::evaluate_evidence_snapshot(
        &arena.repo,
        SESSION,
        None,
        None,
        &record,
    );
    assert!(evidence.describe().contains("interrupted"), "{evidence:?}");
    assert!(!evidence.is_delivery_acceptable());
    let (_, lease) = arena.run_in(&arena.repo, STATUS);
    assert!(lease.starts_with("verification lease: free"), "{lease}");
}

#[test]
fn sigterm_of_verify_runner_preserves_interruption_record() {
    assert_killed_runner_is_recorded(libc::SIGTERM);
}

#[test]
fn sigkill_of_verify_runner_preserves_interruption_record() {
    assert_killed_runner_is_recorded(libc::SIGKILL);
}

#[test]
fn a_separate_watchdog_cannot_interrupt_another_live_runner() {
    let arena = Arena::new();
    let run = CanonicalRun::start(&arena, &arena.repo);
    let path = arena.repo.join(".gwt/skill-state/verification-run.json");
    let before: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let output = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .arg("--verification-watchdog")
        .arg(&arena.repo)
        .arg(before["record_id"].as_str().unwrap())
        .env("HOME", arena.home.path())
        .env("USERPROFILE", arena.home.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let (ok, result) = run.finish();
    assert_eq!(
        before, after,
        "unrelated companion must not change the live record: {output:?}"
    );
    assert!(ok, "{result}");
}

// Issue #5035 AC-6: real two-worktree contention retains successful Heavy
// occurrences on an identical retry. Marker/counter files are bookkeeping,
// so writing them must not change the source fingerprint being verified.
fn continuation_fixture(role: &str, park: bool) {
    let signals = PathBuf::from(std::env::var_os("ADMISSION_CONTINUATION_SIGNALS").unwrap());
    let active = signals.join("heavy-active");
    let guard = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&active)
        .expect("Heavy command fixtures must never overlap");
    let occupied = Instant::now();
    std::fs::write(signals.join(format!("{role}.ready")), "ready").unwrap();
    let work = Instant::now();
    // A fixed amount of fixture work makes avoided execution measurable;
    // ready/release and the observed queue decide ordering, never this delay.
    std::thread::sleep(Duration::from_millis(250));
    let work_ms = work.elapsed().as_millis();
    let deadline = Instant::now() + Duration::from_secs(30);
    while park && !signals.join(format!("{role}.release")).exists() {
        assert!(Instant::now() < deadline, "{role} fixture was not released");
        std::thread::sleep(Duration::from_millis(100));
    }
    let measurement = serde_json::json!({
        "role": role, "work_ms": work_ms, "occupancy_ms": occupied.elapsed().as_millis()
    });
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(signals.join(format!("{role}.executions.jsonl")))
            .unwrap(),
        "{measurement}"
    )
    .unwrap();
    drop(guard);
    std::fs::remove_file(active).unwrap();
}

#[test]
#[ignore = "spawned as the first Heavy continuation command"]
fn admission_continuation_first_command() {
    continuation_fixture("a-first", true);
}

#[test]
#[ignore = "spawned as the second Heavy continuation command"]
fn admission_continuation_second_command() {
    continuation_fixture("a-second", false);
}

#[test]
#[ignore = "spawned as the sibling worktree's canonical holder"]
fn admission_continuation_blocker_command() {
    continuation_fixture("b-heavy", true);
}

fn continuation_command(name: &str) -> String {
    format!(
        "\"{}\" --ignored --exact {name}",
        std::env::current_exe().unwrap().display()
    )
}

fn continuation_run(commands: &[String], max_wait_secs: u64) -> String {
    serde_json::json!({
        "schema_version": 1, "operation": "verify.run",
        "params": {"commands": commands, "max_wait_secs": max_wait_secs}
    })
    .to_string()
}

fn continuation_plan(arena: &Arena, commands: &[String]) -> serde_json::Value {
    let envelope = serde_json::json!({
        "schema_version": 1, "operation": "verify.plan", "params": {"commands": commands}
    });
    let (ok, output) = arena.run_in(&arena.repo, &envelope.to_string());
    assert!(ok, "register continuation plan: {output}");
    continuation_state(&arena.repo, "verification-plan.json")
}

fn continuation_state(worktree: &Path, file: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(worktree.join(".gwt/skill-state").join(file)).unwrap())
        .unwrap()
}

fn continuation_wait_ready(run: &mut CanonicalRun, signals: &Path, role: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !signals.join(format!("{role}.ready")).exists() {
        if run.child.as_mut().unwrap().try_wait().unwrap().is_some() {
            let (_, output) = collect_gwtd(run.child.take().unwrap());
            panic!("continuation run exited before {role}: {output}");
        }
        assert!(Instant::now() < deadline, "{role} did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn continuation_measurements(signals: &Path, role: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(signals.join(format!("{role}.executions.jsonl")))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn continuation_queue_wait(record: &serde_json::Value) -> u64 {
    record["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["admission"]["queue_wait_ms"].as_u64().unwrap())
        .sum()
}

fn continuation_lease_occupancy(home: &Path) -> (usize, u64) {
    let events = std::fs::read_to_string(
        home.join(".gwt/runtime/verification-coordinator/lease-events.jsonl"),
    )
    .unwrap()
    .lines()
    .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
    .collect::<Vec<_>>();
    let acquired = events
        .iter()
        .filter(|event| event["kind"] == "acquired")
        .collect::<Vec<_>>();
    let occupied_ms = acquired
        .iter()
        .map(|start| {
            let end = events
                .iter()
                .find(|event| event["kind"] == "released" && event["lease_id"] == start["lease_id"])
                .expect("every acquired canonical lease must be released");
            end["at_ms"]
                .as_u64()
                .unwrap()
                .saturating_sub(start["at_ms"].as_u64().unwrap())
        })
        .sum();
    (acquired.len(), occupied_ms)
}

fn continuation_evidence(
    home: &Path,
    worktree: &Path,
    plan: &serde_json::Value,
    record: &serde_json::Value,
) -> gwt::cli::verification_record::EvidenceStatus {
    let _home = gwt_core::test_support::ScopedGwtHome::set(home);
    let plan = serde_json::from_value(plan.clone()).unwrap();
    let record = serde_json::from_value(record.clone()).unwrap();
    gwt::cli::verification_record::evaluate_evidence_snapshot(
        worktree,
        SESSION,
        None,
        Some(&plan),
        &record,
    )
}

fn measure_two_worktree_continuation(reregister_plan: bool) -> serde_json::Value {
    let arena = Arena::new();
    // Continuation requires repo-scoped trusted storage. The older lease-only
    // Arena has no origin and therefore deliberately uses mirror-only state.
    git(
        &arena.repo,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/admission-continuation.git",
        ],
    );
    assert!(gwt_core::repo_hash::detect_repo_hash(&arena.repo).is_some());
    let signals = arena.repo.join(".gwt/admission-continuation");
    std::fs::create_dir_all(&signals).unwrap();
    let commands = vec![
        continuation_command("admission_continuation_first_command"),
        continuation_command("admission_continuation_second_command"),
    ];
    let plan = continuation_plan(&arena, &commands);
    let extra_env = [("ADMISSION_CONTINUATION_SIGNALS", signals.as_path())];
    let total = Instant::now();
    let mut first = CanonicalRun {
        child: Some(spawn_gwtd(
            arena.home.path(),
            &arena.repo,
            &continuation_run(&commands, 0),
            &extra_env,
        )),
        release: signals.join("a-first.release"),
    };
    continuation_wait_ready(&mut first, &signals, "a-first");
    let mut blocker = CanonicalRun {
        child: Some(spawn_gwtd(
            arena.home.path(),
            &arena.sibling,
            &continuation_run(
                &[continuation_command(
                    "admission_continuation_blocker_command",
                )],
                30,
            ),
            &extra_env,
        )),
        release: signals.join("b-heavy.release"),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (ok, status) = arena.run_in(&arena.repo, STATUS);
        assert!(ok, "lease status: {status}");
        if status.lines().any(|line| line.starts_with("queue[0]:")) {
            assert!(status.starts_with("verification lease: held"), "{status}");
            break;
        }
        assert!(
            blocker
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none(),
            "sibling contender exited before reserving admission"
        );
        assert!(Instant::now() < deadline, "sibling did not queue: {status}");
        std::thread::sleep(Duration::from_millis(100));
    }
    // B's FIFO reservation must precede A's second Heavy occurrence.
    let (ok, deferred_output) = first.finish();
    let first_attempt_wall_ms = total.elapsed().as_millis();
    assert!(
        !ok && deferred_output.contains("deferred"),
        "{deferred_output}"
    );
    assert!(
        !deferred_output.contains("verify: PASS"),
        "{deferred_output}"
    );
    let predecessor = continuation_state(&arena.repo, "verification-run.json");
    assert_eq!(predecessor["lifecycle"]["status"], "deferred");
    assert_eq!(predecessor["all_passed"], false);
    assert_eq!(predecessor["commands"].as_array().unwrap().len(), 1);
    assert_eq!(predecessor["commands"][0]["exit_code"], 0);
    assert_eq!(predecessor["verification_plan_hash"], plan["content_hash"]);
    assert_eq!(
        predecessor["continuation"]["commands"],
        serde_json::json!(commands)
    );
    assert_eq!(
        predecessor["continuation"]["headed_e2e_commands"],
        serde_json::json!([])
    );
    assert_eq!(
        predecessor["continuation"]["command_indices"],
        serde_json::json!([0])
    );
    assert!(predecessor["commands"][0]["admission"]["lease_id"].is_string());
    assert_eq!(
        continuation_evidence(arena.home.path(), &arena.repo, &plan, &predecessor),
        gwt::cli::verification_record::EvidenceStatus::Deferred
    );
    assert!(!signals.join("a-second.ready").exists());
    continuation_wait_ready(&mut blocker, &signals, "b-heavy");
    let (ok, blocker_output) = blocker.finish();
    assert!(
        ok && blocker_output.contains("verify: PASS"),
        "{blocker_output}"
    );
    let blocker_record = continuation_state(&arena.sibling, "verification-run.json");
    let initial_contention_wall_ms = total.elapsed().as_millis();

    let retry_plan = if reregister_plan {
        let replacement = continuation_plan(&arena, &commands);
        assert_ne!(replacement["content_hash"], plan["content_hash"]);
        replacement
    } else {
        plan
    };
    let retry = Instant::now();
    let (ok, output) = collect_gwtd(spawn_gwtd(
        arena.home.path(),
        &arena.repo,
        &continuation_run(&commands, 0),
        &extra_env,
    ));
    let retry_wall_ms = retry.elapsed().as_millis();
    assert!(ok && output.contains("verify: PASS"), "{output}");
    let record = continuation_state(&arena.repo, "verification-run.json");
    assert_eq!(record["all_passed"], true);
    assert_eq!(record["plan_covered"], true);
    assert_eq!(record["verification_plan_hash"], retry_plan["content_hash"]);
    assert_eq!(record["commands"].as_array().unwrap().len(), 2);
    assert_ne!(record["record_id"], predecessor["record_id"]);
    assert_eq!(
        continuation_evidence(arena.home.path(), &arena.repo, &retry_plan, &record),
        gwt::cli::verification_record::EvidenceStatus::Fresh
    );
    let first_executions = continuation_measurements(&signals, "a-first");
    let second_executions = continuation_measurements(&signals, "a-second");
    let blocker_executions = continuation_measurements(&signals, "b-heavy");
    assert_eq!(second_executions.len(), 1);
    assert_eq!(blocker_executions.len(), 1);
    assert_eq!(
        record["continuation"]["commands"],
        serde_json::json!(commands)
    );
    assert_eq!(
        record["continuation"]["command_indices"],
        serde_json::json!([0, 1])
    );
    if reregister_plan {
        assert_eq!(
            first_executions.len(),
            2,
            "a new plan must run the entire matrix"
        );
        assert_ne!(record["started_at"], predecessor["started_at"]);
        assert!(record["continuation"]["previous"].is_null(), "{record}");
    } else {
        assert_eq!(
            record["worktree_fingerprint"],
            predecessor["worktree_fingerprint"]
        );
        assert_eq!(
            record["continuation"]["authority_hash"],
            predecessor["continuation"]["authority_hash"]
        );
        assert_eq!(
            first_executions.len(),
            1,
            "a successful Heavy occurrence ran twice"
        );
        assert_eq!(record["started_at"], predecessor["started_at"]);
        assert_eq!(record["commands"][0], predecessor["commands"][0]);
        assert_eq!(record["continuation"]["previous"]["reused_commands"], 1);
        assert_eq!(
            record["continuation"]["previous"]["record_id"],
            predecessor["record_id"]
        );
        assert_eq!(
            record["continuation"]["previous"]["content_hash"],
            predecessor["content_hash"]
        );
        assert!(output.contains("continuation"), "{output}");
        assert!(
            output.contains(predecessor["record_id"].as_str().unwrap()),
            "{output}"
        );
    }
    let (_, status) = arena.run_in(&arena.repo, STATUS);
    assert!(status.starts_with("verification lease: free"), "{status}");
    let executions = first_executions
        .iter()
        .chain(&second_executions)
        .chain(&blocker_executions)
        .collect::<Vec<_>>();
    let (heavy_leases, lease_occupancy_ms) = continuation_lease_occupancy(arena.home.path());
    assert_eq!(heavy_leases, executions.len());
    let a_queue_wait_ms = continuation_queue_wait(&record)
        + if reregister_plan {
            continuation_queue_wait(&predecessor)
        } else {
            0
        };
    serde_json::json!({
        "mode": if reregister_plan { "fresh_plan" } else { "continuation" },
        "first_command_executions": first_executions.len(),
        "second_command_executions": second_executions.len(),
        "heavy_executions": executions.len(),
        "heavy_leases": heavy_leases,
        "lease_occupancy_ms": lease_occupancy_ms,
        "fixture_heavy_occupancy_ms": executions.iter().map(|row| row["occupancy_ms"].as_u64().unwrap()).sum::<u64>(),
        "fixture_work_ms": executions.iter().map(|row| row["work_ms"].as_u64().unwrap()).sum::<u64>(),
        "first_attempt_wall_ms": first_attempt_wall_ms,
        "initial_contention_wall_ms": initial_contention_wall_ms,
        "retry_wall_ms": retry_wall_ms,
        "total_wall_ms": total.elapsed().as_millis(),
        "a_queue_wait_ms": a_queue_wait_ms,
        "b_queue_wait_ms": continuation_queue_wait(&blocker_record),
        "predecessor_record_id": predecessor["record_id"],
        "predecessor_content_hash": predecessor["content_hash"],
        "retry_record_id": record["record_id"],
        "retry_content_hash": record["content_hash"]
    })
}

#[test]
fn admission_deferred_two_worktree_retry_reuses_passes_and_measures_saved_execution() {
    let resumed = measure_two_worktree_continuation(false);
    let fresh = measure_two_worktree_continuation(true);
    assert_eq!(resumed["heavy_executions"], 3);
    assert_eq!(fresh["heavy_executions"], 4);
    eprintln!(
        "GWT_ADMISSION_CONTINUATION_MEASUREMENT={}",
        serde_json::json!({"continuation": resumed, "fresh_plan": fresh})
    );
}
