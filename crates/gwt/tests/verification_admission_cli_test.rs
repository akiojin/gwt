//! Issue #3913: `verify.run` host admission across real `gwtd` invocations.
//!
//! The scenario behind the Issue is several agent worktrees of one repository
//! compiling at once, so every test here builds a real repository with a
//! sibling worktree and runs the real binary against it. Heavy load is
//! simulated by re-executing this test binary — which lives under
//! `target/debug/deps/`, exactly like a running test binary of the sibling —
//! parked on an ignored test with the sibling as its working directory.

#![cfg(unix)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::Duration;

use gwt_core::process::hidden_command;
use tempfile::TempDir;

const SESSION: &str = "session-admission-test";

fn gwtd(home: &Path, cwd: &Path, envelope: &str) -> (bool, String) {
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
    let mut child = command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GWT_SESSION_ID", SESSION)
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
        hidden_command(std::env::current_exe().expect("test binary path"))
            .args(["--ignored", "--exact", "fake_heavy_process_parks"])
            .current_dir(&self.sibling)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn fake heavy process")
    }
}

fn verify_run(max_wait_secs: u64) -> String {
    format!(
        r#"{{"schema_version":1,"operation":"verify.run","params":{{"commands":["git --version"],"max_wait_secs":{max_wait_secs}}}}}"#
    )
}

const STATUS: &str = r#"{"schema_version":1,"operation":"verify.lease.status","params":{}}"#;
const ACQUIRE_2M: &str = r#"{"schema_version":1,"operation":"verify.lease.acquire","params":{"ttl_minutes":2,"reason":"admission test"}}"#;

fn field<'a>(output: &'a str, key: &str) -> &'a str {
    output
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&format!("{key}: ")))
        .unwrap_or_else(|| panic!("output has no `{key}` field:\n{output}"))
}

fn kill(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Self-exec target for `spawn_sibling_heavy`.
#[test]
#[ignore = "spawned as the sibling worktree's fake test binary"]
fn fake_heavy_process_parks() {
    std::thread::sleep(Duration::from_secs(120));
}

#[test]
fn verify_run_waits_for_a_sibling_worktree_heavy_process_then_runs() {
    let arena = Arena::new();
    let heavy = arena.spawn_sibling_heavy();
    let drain = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        kill(heavy);
    });

    let (ok, output) = arena.run_in(&arena.repo, &verify_run(60));
    drain.join().unwrap();

    assert!(ok, "the run must start once the sibling drains:\n{output}");
    assert!(output.contains("verify: PASS"), "{output}");
    assert!(
        output.contains("host admission")
            && output.contains("waited")
            && !output.contains("waited 0s"),
        "the admission summary must report a real wait:\n{output}"
    );
    let (_, status) = arena.run_in(&arena.repo, STATUS);
    assert!(
        status.starts_with("verification lease: free"),
        "the in-process lease must be released after the run:\n{status}"
    );
}

#[test]
fn verify_run_defers_when_sibling_heavy_processes_outlast_the_wait_budget() {
    let arena = Arena::new();
    let heavy = arena.spawn_sibling_heavy();

    let (ok, output) = arena.run_in(&arena.repo, &verify_run(1));
    let (_, status) = arena.run_in(&arena.repo, STATUS);
    kill(heavy);

    assert!(!ok, "a busy host must defer the run:\n{output}");
    assert!(output.contains("deferred"), "{output}");
    assert!(output.contains("rerun `verify.run`"), "{output}");
    assert!(
        output.contains("verification_admission_cli_test"),
        "the refusal must name the foreign process:\n{output}"
    );
    assert!(
        !output.contains("verify: PASS") && !output.contains("verify: FAIL"),
        "a deferred run must not produce a record:\n{output}"
    );
    assert!(
        status.starts_with("verification lease: free"),
        "a deferred run must not keep the lease:\n{status}"
    );
}

#[test]
fn verify_run_honors_a_lease_already_held_by_this_worktree() {
    let arena = Arena::new();
    let (ok, granted) = arena.run_in(&arena.repo, ACQUIRE_2M);
    assert!(
        ok && granted.starts_with("verification lease: granted"),
        "{granted}"
    );
    let lease_id = field(&granted, "lease_id").to_string();
    let heavy = arena.spawn_sibling_heavy();

    let (ok, output) = arena.run_in(&arena.repo, &verify_run(1));
    kill(heavy);

    assert!(ok, "the lease holder never waits:\n{output}");
    assert!(output.contains("verify: PASS"), "{output}");
    assert!(output.contains("already held"), "{output}");
    let (_, status) = arena.run_in(&arena.repo, STATUS);
    assert!(
        status.starts_with("verification lease: held") && status.contains(&lease_id),
        "the agent's own lease must survive the run:\n{status}"
    );
    let release = format!(
        r#"{{"schema_version":1,"operation":"verify.lease.release","params":{{"lease_id":"{lease_id}"}}}}"#
    );
    let (ok, released) = arena.run_in(&arena.repo, &release);
    assert!(ok, "{released}");
}

#[test]
fn verify_run_defers_while_another_worktree_holds_the_lease() {
    let arena = Arena::new();
    let (ok, granted) = arena.run_in(&arena.sibling, ACQUIRE_2M);
    assert!(
        ok && granted.starts_with("verification lease: granted"),
        "{granted}"
    );
    let lease_id = field(&granted, "lease_id").to_string();
    let holder_target = field(&granted, "target").to_string();

    let (ok, output) = arena.run_in(&arena.repo, &verify_run(1));

    assert!(
        !ok,
        "another worktree's lease must defer the run:\n{output}"
    );
    assert!(output.contains("deferred"), "{output}");
    assert!(
        output.contains(&holder_target),
        "the refusal must name the holder:\n{output}"
    );
    let release = format!(
        r#"{{"schema_version":1,"operation":"verify.lease.release","params":{{"lease_id":"{lease_id}"}}}}"#
    );
    let (ok, released) = arena.run_in(&arena.sibling, &release);
    assert!(ok, "{released}");
}
