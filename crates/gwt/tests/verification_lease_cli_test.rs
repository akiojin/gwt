//! SPEC #3576 T-008: `verify.lease.*` round-trip across separate `gwtd`
//! invocations.
//!
//! The lease has to outlive the process that asked for it — that is the whole
//! point of replacing the manual Board token protocol — so every assertion
//! here runs the real `gwtd` binary and observes state through a *later*
//! invocation, never through in-process handles.

use std::io::Write;
use std::path::Path;
use std::process::Stdio;

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
fn verification_lease_round_trips_across_gwtd_invocations() {
    let arena = Arena::new();

    let free = arena.run(STATUS);
    assert_eq!(headline(&free), "verification lease: free");

    let granted = arena.run(ACQUIRE_2M);
    assert_eq!(
        headline(&granted),
        "verification lease: granted",
        "an uncontended host must grant the lease:\n{granted}"
    );
    let lease_id = field(&granted, "lease_id").to_string();
    assert!(!lease_id.is_empty());

    // A *separate* process observes the lease, which only holds if the
    // acquiring invocation left a live holder behind.
    let held = arena.run(STATUS);
    assert_eq!(headline(&held), "verification lease: held", "{held}");
    assert_eq!(field(&held, "lease_id"), lease_id);
    assert_eq!(field(&held, "expired"), "false");
    let remaining = field_u64(&held, "remaining_ms");
    assert!(
        remaining > 0 && remaining <= 2 * 60 * 1000,
        "remaining TTL must reflect the requested 2 minutes, got {remaining}"
    );

    let extended = arena.run(&format!(
        r#"{{"schema_version":1,"operation":"verify.lease.extend","params":{{"lease_id":"{lease_id}","ttl_minutes":30}}}}"#
    ));
    assert_eq!(
        headline(&extended),
        "verification lease: extended",
        "{extended}"
    );
    assert!(
        field_u64(&extended, "remaining_ms") > remaining,
        "extend must push the deadline out:\n{extended}"
    );

    let released = arena.run(&format!(
        r#"{{"schema_version":1,"operation":"verify.lease.release","params":{{"lease_id":"{lease_id}","reason":"round-trip test done"}}}}"#
    ));
    assert_eq!(
        headline(&released),
        "verification lease: released",
        "{released}"
    );

    let free_again = arena.run(STATUS);
    assert_eq!(
        headline(&free_again),
        "verification lease: free",
        "release must free the host-wide lease:\n{free_again}"
    );

    // FR-5: the ledger records the whole lifecycle for this lease id.
    let events = std::fs::read_to_string(
        arena
            .home
            .path()
            .join(".gwt/runtime/index-coordinator/lease-events.jsonl"),
    )
    .expect("lease event log");
    for kind in ["acquired", "extended", "released"] {
        assert!(
            events
                .lines()
                .any(|line| line.contains(&lease_id) && line.contains(kind)),
            "lease event log must record `{kind}` for {lease_id}:\n{events}"
        );
    }
}

#[test]
fn second_worktree_is_refused_while_the_lease_is_held() {
    let arena = Arena::new();
    let other_worktree = tempfile::tempdir().expect("second worktree");

    let granted = arena.run(ACQUIRE_2M);
    assert_eq!(headline(&granted), "verification lease: granted");
    let lease_id = field(&granted, "lease_id").to_string();

    // AC-1: a different worktree on the same host owns a different target
    // job, so only the host-wide lease can refuse it.
    let (ok, refused) = gwtd(arena.home.path(), other_worktree.path(), ACQUIRE_2M);
    assert!(ok, "a refused acquisition is a normal answer:\n{refused}");
    assert_eq!(
        headline(&refused),
        "verification lease: unavailable",
        "the second claimant must be refused:\n{refused}"
    );
    assert_eq!(
        field(&refused, "lease_id"),
        lease_id,
        "the refusal must name the current holder:\n{refused}"
    );
    assert!(
        field_u64(&refused, "remaining_ms") > 0,
        "the refusal must report the wait estimate:\n{refused}"
    );

    arena.run(&format!(
        r#"{{"schema_version":1,"operation":"verify.lease.release","params":{{"lease_id":"{lease_id}"}}}}"#
    ));

    // Once released, the previously refused worktree can take it.
    let (ok, granted_now) = gwtd(arena.home.path(), other_worktree.path(), ACQUIRE_2M);
    assert!(ok, "{granted_now}");
    assert_eq!(
        headline(&granted_now),
        "verification lease: granted",
        "{granted_now}"
    );
    let second_id = field(&granted_now, "lease_id").to_string();
    gwtd(
        arena.home.path(),
        other_worktree.path(),
        &format!(
            r#"{{"schema_version":1,"operation":"verify.lease.release","params":{{"lease_id":"{second_id}"}}}}"#
        ),
    );
}
