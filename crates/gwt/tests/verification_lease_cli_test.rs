//! SPEC #3576: manual acquisition is retired. Status and release remain
//! compatible with pre-upgrade holders, without creating a new idle holder.

use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use gwt_core::process::hidden_command;
use tempfile::TempDir;

fn gwtd(home: &Path, cwd: &Path, envelope: &str) -> (bool, String) {
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
    ] {
        command.env_remove(key);
    }
    let mut child = command
        .env("HOME", home)
        .env("USERPROFILE", home)
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
    assert!(
        !arena
            .home
            .path()
            .join(".gwt/runtime/index-coordinator")
            .exists(),
        "refusal must not create control, holder, ticket, or reservation state"
    );
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
        assert!(
            !arena
                .home
                .path()
                .join(".gwt/runtime/index-coordinator")
                .exists(),
            "refusal must leave existing leases untouched and create no new state"
        );
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
    let key = TargetKey::verification("repo", "legacy");
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

#[test]
fn releasing_an_index_lease_requests_a_yield_instead_of_failing() {
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
        .expect("index job takes the idle host");
    let lease_id = index_lease.id().to_string();

    let (ok, output) = gwtd(
        arena.home.path(),
        arena.worktree.path(),
        &format!(
            r#"{{"schema_version":1,"operation":"verify.lease.release","params":{{"lease_id":"{lease_id}","reason":"PM arbitration"}}}}"#
        ),
    );
    assert!(
        ok,
        "arbitrating an index lease is a normal answer:\n{output}"
    );
    assert_eq!(
        headline(&output),
        "verification lease: yield requested",
        "{output}"
    );
    assert_eq!(field(&output, "holder_kind"), "index", "{output}");
    assert_eq!(
        field_u64(&output, "pending"),
        1,
        "the arbitration leaves a reservation the runner yields to:\n{output}"
    );
    assert!(
        coordinator
            .pending_higher_priority(JobPriority::Background)
            .unwrap(),
        "the index runner must observe a higher-priority pending claimant"
    );
    drop(index_lease);
    index_guard
        .complete(gwt_core::index_coordinator::JobOutcome::Completed)
        .unwrap();
}
