//! SPEC #3576: manual acquisition is retired. Status and release remain
//! compatible with pre-upgrade holders, without creating a new idle holder.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use gwt_core::process::hidden_command;
use tempfile::TempDir;

fn gwtd(home: &Path, cwd: &Path, envelope: &str) -> (bool, String) {
    collect_gwtd(spawn_gwtd(home, cwd, envelope))
}

fn spawn_gwtd(home: &Path, cwd: &Path, envelope: &str) -> Child {
    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
    for key in [
        "GWT_BIN_PATH",
        "GWT_BROWSER_URL_FILE",
        "GWT_HOOK_BIN",
        "GWT_PROJECT_ROOT",
        "GWT_REPO_HASH",
        "GWT_SESSION_ID",
        "GWT_SESSION_KIND",
        "GWT_SESSION_RUNTIME_PATH",
        "GWT_WORKTREE_HASH",
        "GWT_AUTONOMOUS_ISSUE",
        "GWT_AUTONOMOUS_EXECUTION",
        "GWT_HOOK_FORWARD_URL",
        "GWT_HOOK_FORWARD_TOKEN",
    ] {
        command.env_remove(key);
    }
    let mut child = command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GWT_SESSION_ID", "session-lease-cli-test")
        // Issue #4409: this suite is about the lease surface, not about where
        // verification is hosted. Declaring the host keeps the result from
        // depending on the priority the test runner happened to inherit.
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

fn collect_gwtd(child: Child) -> (bool, String) {
    let output = child.wait_with_output().expect("await gwtd");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // Operations answer inside a JSON envelope; the assertions below care
    // about the operation payload, not the transport.
    let payload = serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .and_then(|envelope| {
            envelope
                .get("output")
                .and_then(|output| output.as_str())
                .map(str::to_string)
        })
        .unwrap_or(stdout);
    (output.status.success(), format!("{payload}{stderr}"))
}

/// Own only the spawned fixture process, including on an assertion failure.
struct WaitingRun(Option<Child>);

impl Drop for WaitingRun {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Read one `key: value` line out of the operation output.
fn field<'a>(output: &'a str, key: &str) -> &'a str {
    output
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&format!("{key}: ")))
        .unwrap_or_else(|| panic!("output has no `{key}` field:\n{output}"))
}

fn field_u64(output: &str, key: &str) -> u64 {
    field(output, key)
        .parse()
        .unwrap_or_else(|_| panic!("`{key}` must be numeric:\n{output}"))
}

fn headline(output: &str) -> &str {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

struct Arena {
    home: TempDir,
    worktree: TempDir,
}

impl Arena {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().expect("home tempdir"),
            worktree: tempfile::tempdir().expect("worktree tempdir"),
        }
    }

    fn run(&self, envelope: &str) -> String {
        let (ok, output) = gwtd(self.home.path(), self.worktree.path(), envelope);
        assert!(ok, "gwtd failed for {envelope}:\n{output}");
        output
    }
}

const ACQUIRE_2M: &str = r#"{"schema_version":1,"operation":"verify.lease.acquire","params":{"ttl_minutes":2,"reason":"round-trip test"}}"#;
const STATUS: &str = r#"{"schema_version":1,"operation":"verify.lease.status","params":{}}"#;

#[test]
fn manual_acquire_is_rejected_without_creating_a_holder() {
    let arena = Arena::new();
    let (ok, output) = gwtd(arena.home.path(), arena.worktree.path(), ACQUIRE_2M);
    // Keep the RED run from leaving a detached holder behind.
    if ok && headline(&output) == "verification lease: granted" {
        let lease_id = field(&output, "lease_id");
        arena.run(&format!(
            r#"{{"schema_version":1,"operation":"verify.lease.release","params":{{"lease_id":"{lease_id}"}}}}"#
        ));
    }
    assert!(!ok, "manual acquisition must be retired: {output}");
    assert!(output.contains("verify.run"), "{output}");
    assert!(output.contains("directly"), "{output}");
    for root in ["index-coordinator", "verification-coordinator"] {
        assert!(
            !arena.home.path().join(".gwt/runtime").join(root).exists(),
            "refusal must not create control, holder, ticket, or reservation state ({root})"
        );
    }
}

#[test]
fn manual_hold_and_extend_are_rejected_without_creating_state() {
    for request in [
        r#"{"schema_version":1,"operation":"verify.lease.hold","params":{"ttl_minutes":1,"control":"outside-runtime"}}"#,
        r#"{"schema_version":1,"operation":"verify.lease.extend","params":{"ttl_minutes":1,"lease_id":"old-lease"}}"#,
    ] {
        let arena = Arena::new();
        let (ok, output) = gwtd(arena.home.path(), arena.worktree.path(), request);
        assert!(!ok, "manual holding must be retired: {output}");
        assert!(output.contains("verify.run"), "{output}");
        for root in ["index-coordinator", "verification-coordinator"] {
            assert!(
                !arena.home.path().join(".gwt/runtime").join(root).exists(),
                "refusal must leave existing leases untouched and create no new state ({root})"
            );
        }
    }
}

fn index_coordinator(home: &Path) -> gwt_core::index_coordinator::IndexCoordinator {
    gwt_core::index_coordinator::IndexCoordinator::open(
        gwt_core::index_coordinator::coordinator_root_from(&home.join(".gwt")),
    )
    .expect("open coordinator under the test home")
}

#[test]
fn legacy_holder_can_still_be_observed_and_released() {
    use gwt_core::index_coordinator::{JobAdmission, JobOutcome, JobPriority, TargetKey};

    let arena = Arena::new();
    let coordinator = index_coordinator(arena.home.path());
    let project = gwt_core::paths::project_scope_hash(arena.worktree.path());
    let key = TargetKey::verification(project.as_str(), "legacy");
    let JobAdmission::Owner(guard) = coordinator
        .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(5))
        .unwrap()
    else {
        panic!("legacy target must be free");
    };
    let lease = guard
        .acquire_heavy_with_ttl(Duration::from_secs(5), Duration::from_secs(60))
        .unwrap();
    let lease_id = lease.id().to_string();
    let control = arena
        .home
        .path()
        .join(".gwt/runtime/index-coordinator/verification.control/legacy-holder");
    std::fs::create_dir_all(&control).unwrap();
    std::fs::write(
        control.join("outcome.json"),
        serde_json::json!({"granted":true,"held":true,"lease_id":lease_id}).to_string(),
    )
    .unwrap();
    let held = arena.run(STATUS);
    assert_eq!(headline(&held), "verification lease: held");
    assert_eq!(field(&held, "lease_id"), lease_id);
    assert_eq!(field(&held, "holder_project_relation"), "same_project");
    assert_eq!(
        field(&held, "holder_intervention"),
        "canonical_release_only"
    );
    let (ok, refused) = gwtd(
        arena.home.path(),
        arena.worktree.path(),
        &format!(
            r#"{{"schema_version":1,"operation":"verify.lease.extend","params":{{"lease_id":"{lease_id}","ttl_minutes":30}}}}"#
        ),
    );
    assert!(!ok, "legacy holders must not be extended: {refused}");
    assert!(refused.contains("verify.run"), "{refused}");
    let still_held = arena.run(STATUS);
    assert_eq!(field(&still_held, "lease_id"), lease_id);
    assert_eq!(
        field(&still_held, "expires_at_ms"),
        field(&held, "expires_at_ms")
    );

    // Model the pre-upgrade holder's existing release channel, with a real
    // kernel lease. The new binary must never spawn a replacement holder.
    let release_path = control.join("release");
    let old_holder = std::thread::spawn(move || {
        // Waiting for the path is sound only because the release channel is
        // published by rename (Issue #4360): the file appears already holding
        // its whole payload. An empty reason is a legitimate value here, so
        // "non-empty" cannot be the readiness signal — do not weaken the
        // writer back to a plain `fs::write`.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !release_path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let reason = std::fs::read_to_string(release_path).unwrap();
        drop(lease);
        guard.complete(JobOutcome::Completed).unwrap();
        reason
    });
    let released = arena.run(&format!(
        r#"{{"schema_version":1,"operation":"verify.lease.release","params":{{"lease_id":"{lease_id}","reason":"upgrade to canonical runs"}}}}"#
    ));
    assert_eq!(old_holder.join().unwrap(), "upgrade to canonical runs");
    assert_eq!(headline(&released), "verification lease: released");
    assert_eq!(headline(&arena.run(STATUS)), "verification lease: free");
    assert!(!control.exists(), "legacy control state must be cleaned up");
}

/// Issue #4285 AC-4: an index job or query encode on the model lane is not a
/// verification holder. `verify.lease.status` reads the verification lane
/// only, and `verify.lease.release` no longer arbitrates index leases.
#[test]
fn an_index_lease_is_invisible_to_the_verification_lane() {
    use gwt_core::index_coordinator::{JobAdmission, JobPriority, TargetKey};
    use std::time::Duration;

    let arena = Arena::new();
    let coordinator = index_coordinator(arena.home.path());
    let index_key = TargetKey::repo_shared("99a8660247f5bc49", "issues");
    let JobAdmission::Owner(index_guard) = coordinator
        .request_job(&index_key, JobPriority::Background, Duration::from_secs(5))
        .expect("request index job")
    else {
        panic!("index target must be free");
    };
    let index_lease = index_guard
        .acquire_heavy_with_ttl(
            Duration::from_secs(5),
            gwt_core::index_coordinator::INDEX_HEAVY_LEASE_TTL,
        )
        .expect("index job takes the idle model lane");
    let lease_id = index_lease.id().to_string();

    let status = arena.run(STATUS);
    assert_eq!(
        headline(&status),
        "verification lease: free",
        "an index holder must not read as a verification holder:
{status}"
    );
    assert_eq!(field_u64(&status, "pending"), 0, "{status}");

    let (ok, output) = gwtd(
        arena.home.path(),
        arena.worktree.path(),
        &format!(
            r#"{{"schema_version":1,"operation":"verify.lease.release","params":{{"lease_id":"{lease_id}","reason":"PM arbitration"}}}}"#
        ),
    );
    assert!(
        !ok,
        "an index lease is not a verification lease:
{output}"
    );
    assert!(output.contains("no live verification lease"), "{output}");
    assert!(
        !coordinator
            .pending_higher_priority(JobPriority::Background)
            .unwrap(),
        "the release must leave no reservation behind on the model lane"
    );
    drop(index_lease);
    index_guard
        .complete(gwt_core::index_coordinator::JobOutcome::Completed)
        .unwrap();
}

/// Issue #4280 AC-2 / AC-4: a waiter reads how far the holder's `verify.run`
/// has got (commands left and a paced estimate) instead of the TTL alone.
#[test]
fn status_reports_the_verification_holders_remaining_commands() {
    use gwt_core::index_coordinator::{JobAdmission, JobOutcome, JobPriority, TargetKey};

    let arena = Arena::new();
    let coordinator = index_coordinator(arena.home.path());
    let key = TargetKey::verification("repo", "holder");
    let JobAdmission::Owner(guard) = coordinator
        .request_job(&key, JobPriority::ManualRebuild, Duration::from_secs(5))
        .unwrap()
    else {
        panic!("holder target must be free");
    };
    let lease = guard
        .acquire_heavy_with_ttl(Duration::from_secs(5), Duration::from_secs(2_700))
        .unwrap();
    // Two of five commands finished at 45 s each: three are left.
    lease.publish_progress(2, 5, 45_000).unwrap();

    let status = arena.run(STATUS);
    assert_eq!(headline(&status), "verification lease: held", "{status}");
    assert_eq!(field(&status, "holder_kind"), "verification", "{status}");
    assert_eq!(field_u64(&status, "remaining_batches"), 3, "{status}");
    assert_eq!(
        field_u64(&status, "estimated_remaining_ms"),
        135_000,
        "{status}"
    );

    drop(lease);
    guard.complete(JobOutcome::Completed).unwrap();
}

/// Issue #4998: observe kernel release before retrying the legacy host lock.
fn assert_heavy_lock_released(coordinator: &gwt_core::index_coordinator::IndexCoordinator) {
    let path = coordinator.heavy_lock_path();
    let probe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match fs2::FileExt::try_lock_exclusive(&probe) {
            Ok(()) => {
                // Explicit unlock also releases any fork-inherited copy of
                // this probe's own file description.
                fs2::FileExt::unlock(&probe).unwrap();
                return;
            }
            Err(error) => {
                assert_eq!(
                    error.raw_os_error(),
                    fs2::lock_contended_error().raw_os_error(),
                    "release probe failed: {error}; {}",
                    path.display()
                );
                assert!(
                    Instant::now() < deadline,
                    "lock still held after release: {}",
                    path.display()
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Issue #4969: cross-process admission must preserve the same arrival during
/// polling, after a Windows status-probe lock, and across deferred resubmission.
fn assert_deferred_fifo_across_processes(max_wait_secs: u64) {
    use gwt_core::index_coordinator::{
        verification_coordinator_root_from, JobAdmission, JobOutcome, JobPriority, TargetKey,
    };

    let arena = Arena::new();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "init",
        ],
    ] {
        assert!(hidden_command("git")
            .args(args)
            .current_dir(arena.worktree.path())
            .status()
            .unwrap()
            .success());
    }
    let coordinator = gwt_core::index_coordinator::IndexCoordinator::open(
        verification_coordinator_root_from(&arena.home.path().join(".gwt")),
    )
    .unwrap();
    let project = gwt_core::paths::project_scope_hash(arena.worktree.path());
    let holder = TargetKey::verification(project.as_str(), "holder");
    let JobAdmission::Owner(guard) = coordinator
        .request_job(&holder, JobPriority::ManualRebuild, Duration::ZERO)
        .unwrap()
    else {
        panic!("private fixture holder must own its target");
    };
    let lease = guard
        .acquire_heavy_with_ttl(Duration::ZERO, Duration::from_secs(3600))
        .unwrap();
    let request = |wait| {
        serde_json::json!({
            "schema_version": 1,
            "operation": "verify.run",
            "params": {"commands": ["git --version"], "max_wait_secs": wait}
        })
        .to_string()
    };
    let (ok, initial) = gwtd(arena.home.path(), arena.worktree.path(), &request(0));
    assert!(!ok && initial.contains("deferred"), "{initial}");
    assert!(initial.contains("next_turn_reserved: yes"), "{initial}");
    let early = coordinator.heavy_lease_status().unwrap().queue[0].clone();
    let key = TargetKey::verification(
        project.as_str(),
        gwt_core::worktree_hash::compute_worktree_hash(arena.worktree.path())
            .unwrap()
            .as_str(),
    );
    assert_eq!(early.target.as_deref(), Some(key.file_stem().as_str()));
    let later = TargetKey::verification(project.as_str(), "later");
    coordinator
        .reserve_heavy(
            &later,
            JobPriority::ManualRebuild,
            Duration::from_secs(max_wait_secs + 60),
            Some("later same-priority fixture"),
        )
        .unwrap();

    #[cfg(windows)]
    {
        // Reproduce the real status sweep's mandatory probe lock. A fresh
        // gwtd process must treat this read as unknown, never as no reservation.
        let probe = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(coordinator.heavy_reservation_path(&key))
            .unwrap();
        fs2::FileExt::lock_exclusive(&probe).unwrap();
        let (ok, contended) = gwtd(arena.home.path(), arena.worktree.path(), &request(0));
        fs2::FileExt::unlock(&probe).unwrap();
        assert!(!ok && contended.contains("deferred"), "{contended}");
        assert!(
            contended.contains("next_turn_reserved: unknown"),
            "{contended}"
        );
        assert!(!contended.contains("next_turn_reserved: no"), "{contended}");
        let queue = coordinator.heavy_lease_status().unwrap().queue;
        assert_eq!(queue[0].target, early.target, "{queue:?}");
        assert_eq!(queue[0].queued_at_ms, early.queued_at_ms, "{queue:?}");
    }

    let started = Instant::now();
    let mut waiting = WaitingRun(Some(spawn_gwtd(
        arena.home.path(),
        arena.worktree.path(),
        &request(max_wait_secs),
    )));
    loop {
        assert!(started.elapsed() < Duration::from_secs(max_wait_secs + 60));
        // A concurrent status probe can briefly make a registration unreadable
        // on Windows. Observe the next complete snapshot, as admission does.
        let Ok(status) = coordinator.heavy_lease_status() else {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        assert_eq!(status.lease_id.as_deref(), Some(lease.id()));
        assert_eq!(status.queue[0].target, early.target, "{status:?}");
        assert_eq!(
            status.queue[0].queued_at_ms, early.queued_at_ms,
            "{status:?}"
        );
        assert_eq!(
            status.queue[1].target.as_deref(),
            Some(later.file_stem().as_str())
        );
        if waiting.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let (ok, deferred) = collect_gwtd(waiting.0.take().unwrap());
    assert!(!ok && deferred.contains("deferred"), "{deferred}");
    assert!(started.elapsed() >= Duration::from_secs(max_wait_secs));
    assert!(deferred.contains("next_turn_reserved: yes"), "{deferred}");
    assert!(deferred.contains("queue_position: 1"), "{deferred}");
    let status = arena.run(STATUS);
    assert_eq!(field(&status, "lease_id"), lease.id());
    assert_eq!(field(&status, "holder_intervention"), "forbidden");
    assert_eq!(field_u64(&status, "pending"), 2);
    let queue = coordinator.heavy_lease_status().unwrap().queue;
    for (position, entry) in queue.iter().enumerate() {
        assert!(
            field(&status, &format!("queue[{position}]")).starts_with(&format!(
                "target={} priority=manual-rebuild queued_at_ms={} ",
                entry.target.as_deref().unwrap(),
                entry.queued_at_ms
            )),
            "{status}"
        );
    }

    // A different fixture target takes the host while this claimant remains
    // queued. Handoff sweeps must not assign the claimant a new arrival.
    lease.release().unwrap();
    guard.complete(JobOutcome::Completed).unwrap();
    assert_heavy_lock_released(&coordinator);
    let successor = TargetKey::verification(project.as_str(), "successor-holder");
    let JobAdmission::Owner(guard) = coordinator
        .request_job(&successor, JobPriority::InteractiveSearch, Duration::ZERO)
        .unwrap()
    else {
        panic!("successor fixture must own its target");
    };
    let lease = guard.acquire_heavy(Duration::ZERO).unwrap();
    let mixed_request = serde_json::json!({
        "schema_version": 1,
        "operation": "verify.run",
        "params": {
            "commands": [format!("\"{}\" fmt --version", env!("CARGO")), "git --version".to_string()],
            "max_wait_secs": 0
        }
    }).to_string();
    let (ok, mixed) = gwtd(arena.home.path(), arena.worktree.path(), &mixed_request);
    assert!(!ok && mixed.contains("next_turn_reserved: yes"), "{mixed}");
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            arena
                .worktree
                .path()
                .join(".gwt/skill-state/verification-run.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["commands"][0]["exit_code"], 0, "{record}");
    assert!(record["commands"][0]["admission"].is_null(), "{record}");
    let queue = coordinator.heavy_lease_status().unwrap().queue;
    assert_eq!(queue[0].target, early.target);
    assert_eq!(queue[0].queued_at_ms, early.queued_at_ms);

    // Another process is materialized after the first one has returned.
    let (ok, resubmitted) = gwtd(arena.home.path(), arena.worktree.path(), &request(0));
    assert!(!ok && resubmitted.contains("deferred"), "{resubmitted}");
    assert!(
        resubmitted.contains("next_turn_reserved: yes"),
        "{resubmitted}"
    );
    assert!(resubmitted.contains("queue_position: 1"), "{resubmitted}");
    let queue = coordinator.heavy_lease_status().unwrap().queue;
    assert_eq!(queue[0].target, early.target);
    assert_eq!(queue[0].queued_at_ms, early.queued_at_ms);
    assert_eq!(queue[1].target.as_deref(), Some(later.file_stem().as_str()));

    // Only the owning fixture releases the holder. The early target takes the
    // free host while the later reservation remains queued behind it.
    lease.release().unwrap();
    guard.complete(JobOutcome::Completed).unwrap();
    assert_heavy_lock_released(&coordinator);
    let (ok, admitted) = gwtd(arena.home.path(), arena.worktree.path(), &request(0));
    assert!(ok && admitted.contains("verify: PASS"), "{admitted}");
    assert!(!coordinator.heavy_reservation_path(&key).exists());
    assert!(coordinator.heavy_reservation_path(&later).exists());
}

#[test]
fn deferred_fifo_survives_status_read_contention_and_new_processes() {
    assert_deferred_fifo_across_processes(6);
}

#[test]
#[ignore = "Issue #4969 acceptance: actual 1500-second holder-busy admission"]
fn deferred_fifo_survives_actual_1500_second_wait_and_new_processes() {
    assert_deferred_fifo_across_processes(1500);
}
