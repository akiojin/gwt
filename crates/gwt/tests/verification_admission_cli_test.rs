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
