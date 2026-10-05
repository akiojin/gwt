//! Issue #4823: actual derived matrices share Light commands but serialize Heavy commands.
//! A tiny Cargo stand-in keeps the production command lines and process boundaries,
//! while readiness/release handshakes make concurrency independent of machine speed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use gwt_core::process::hidden_command;
use serde_json::{json, Value};
use tempfile::TempDir;

const FIXTURE: &str = r#"
use std::{env, fs, path::PathBuf, thread, time::{Duration, Instant}};
fn main() {
    let command = env::args().nth(1).unwrap();
    let root = PathBuf::from(env::var_os("ADMISSION_SIGNALS").unwrap());
    let worker = env::var("ADMISSION_WORKER").unwrap();
    let marker = root.join(format!("{worker}-{command}"));
    let heavy = matches!(command.as_str(), "clippy" | "test");
    let lock = root.join("heavy-active");
    let guard = heavy.then(|| fs::OpenOptions::new().write(true).create_new(true)
        .open(&lock).expect("Heavy commands overlapped"));
    fs::write(marker.with_extension("pid"), std::process::id().to_string()).unwrap();
    fs::rename(marker.with_extension("pid"), marker.with_extension("ready")).unwrap();
    if matches!(command.as_str(), "fmt" | "clippy") {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !marker.with_extension("release").exists() && !root.join("release-all").exists() {
            assert!(Instant::now() < deadline, "fixture was not released");
            thread::sleep(Duration::from_millis(100));
        }
    }
    drop(guard);
    if heavy { fs::remove_file(lock).unwrap(); }
    fs::write(marker.with_extension("done"), "done").unwrap();
}
"#;

fn git(cwd: &Path, args: &[&str]) {
    let output = hidden_command("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

struct Arena {
    root: TempDir,
    home: PathBuf,
    signals: PathBuf,
    bin: PathBuf,
    first: PathBuf,
    second: PathBuf,
}

impl Arena {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let signals = root.path().join("signals");
        let bin = root.path().join("bin");
        let first = root.path().join("first");
        let second = root.path().join("second");
        for path in [&home, &signals, &bin, &first] {
            std::fs::create_dir_all(path).unwrap();
        }
        let source = root.path().join("cargo_fixture.rs");
        std::fs::write(&source, FIXTURE).unwrap();
        let output = hidden_command("rustc")
            .arg(&source)
            .arg("-o")
            .arg(bin.join(format!("cargo{}", std::env::consts::EXE_SUFFIX)))
            .output()
            .unwrap();
        assert!(output.status.success(), "compile Cargo fixture: {output:?}");

        git(&first, &["init", "-q", "-b", "fixture-first"]);
        git(&first, &["config", "user.email", "test@example.com"]);
        git(&first, &["config", "user.name", "Test"]);
        std::fs::create_dir_all(first.join("src")).unwrap();
        std::fs::write(
            first.join("Cargo.toml"),
            "[package]\nname = \"admission-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(first.join("src/lib.rs"), "pub fn baseline() {}\n").unwrap();
        git(&first, &["add", "."]);
        git(&first, &["commit", "-qm", "fixture baseline"]);
        git(
            &first,
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        // Test-only repository, never the agent's managed worktree.
        git(
            &first,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "fixture-second",
                second.to_str().unwrap(),
            ],
        );
        for worktree in [&first, &second] {
            std::fs::write(worktree.join("src/lib.rs"), "pub fn changed() {}\n").unwrap();
        }
        Self {
            root,
            home,
            signals,
            bin,
            first,
            second,
        }
    }

    fn spawn(&self, worktree: &Path, worker: &str, operation: &str, params: Value) -> Child {
        // An optional old binary makes the same handshake a before/after probe.
        let binary = std::env::var_os("GWT_ADMISSION_TEST_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_gwtd").into());
        let mut command = hidden_command(binary);
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
        let mut paths = vec![self.bin.clone()];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let mut child = command
            .env("PATH", std::env::join_paths(paths).unwrap())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("GWT_HOME", self.home.join(".gwt"))
            .env("GWT_SESSION_ID", format!("admission-{worker}"))
            .env("GWT_VERIFY_SPAWN_HOST", "inherit")
            .env("ADMISSION_SIGNALS", &self.signals)
            .env("ADMISSION_WORKER", worker)
            .current_dir(worktree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let envelope = json!({"schema_version": 1, "operation": operation, "params": params});
        child
            .stdin
            .take()
            .unwrap()
            .write_all(envelope.to_string().as_bytes())
            .unwrap();
        child
    }

    fn marker(&self, worker: &str, command: &str, state: &str) -> PathBuf {
        self.signals.join(format!("{worker}-{command}.{state}"))
    }

    fn release(&self, worker: &str, command: &str) {
        std::fs::write(self.marker(worker, command, "release"), "release").unwrap();
    }

    fn plan(&self, worktree: &Path, worker: &str) -> Vec<String> {
        let output = collect(self.spawn(worktree, worker, "verify.plan", json!({"derive": true})));
        assert!(output.contains("derived"), "{output}");
        // Read the canonical result, not a parallel test-only matrix.
        let plan: Value = serde_json::from_slice(
            &std::fs::read(worktree.join(".gwt/skill-state/verification-plan.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(plan["derived"], true, "{plan}");
        plan["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect()
    }
}

fn collect(child: Child) -> String {
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    let payload = serde_json::from_str::<Value>(&stdout)
        .ok()
        .and_then(|envelope| envelope["output"].as_str().map(str::to_owned))
        .unwrap_or_else(|| stdout.into_owned());
    format!("{payload}\n{stderr}")
}

struct Run<'a> {
    arena: &'a Arena,
    child: Option<Child>,
}

impl Run<'_> {
    fn wait_ready(&mut self, worker: &str, command: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.arena.marker(worker, command, "ready").exists() {
            if self.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                panic!(
                    "run exited before {worker}-{command}: {}",
                    collect(self.child.take().unwrap())
                );
            }
            assert!(
                Instant::now() < deadline,
                "{worker}-{command} did not start"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn finish(mut self) {
        let output = collect(self.child.take().unwrap());
        assert!(output.contains("verify: PASS"), "{output}");
    }
}

impl Drop for Run<'_> {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Also unpark grandchildren on assertion failure.
            let _ = std::fs::write(self.arena.signals.join("release-all"), "release");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn derived_matrices_overlap_light_commands_and_serialize_heavy_commands() {
    let arena = Arena::new();
    let first_commands = arena.plan(&arena.first, "a");
    let second_commands = arena.plan(&arena.second, "b");
    assert_eq!(first_commands, second_commands);
    assert_eq!(first_commands.len(), 4, "{first_commands:?}");
    let mut first = Run {
        arena: &arena,
        child: Some(arena.spawn(
            &arena.first,
            "a",
            "verify.run",
            json!({
                "commands": first_commands, "max_wait_secs": 30
            }),
        )),
    };
    first.wait_ready("a", "fmt");
    let started = Instant::now();
    let mut second = Run {
        arena: &arena,
        child: Some(arena.spawn(
            &arena.second,
            "b",
            "verify.run",
            json!({
                "commands": second_commands, "max_wait_secs": 30
            }),
        )),
    };
    second.wait_ready("b", "fmt");
    eprintln!(
        "second Light command wait: {:?}; first Light still active",
        started.elapsed()
    );
    assert!(!arena.marker("a", "fmt", "done").exists());
    #[cfg(windows)]
    for worker in ["a", "b"] {
        let pid = std::fs::read_to_string(arena.marker(worker, "fmt", "ready"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            gwt_core::process_tree::process_priority_class(pid).unwrap(),
            gwt_core::process_tree::ProcessPriorityClass::Normal
        );
    }
    arena.release("a", "fmt");
    first.wait_ready("a", "clippy");
    arena.release("b", "fmt");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = collect(arena.spawn(&arena.second, "b", "verify.lease.status", json!({})));
        if status.contains("queue[0]") {
            eprintln!("Heavy contender queue observed: {status}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Heavy contender did not queue: {status}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!arena.marker("b", "clippy", "ready").exists());
    arena.release("a", "clippy");
    second.wait_ready("b", "clippy");
    assert!(arena.marker("a", "clippy", "done").exists());
    arena.release("b", "clippy");
    first.finish();
    second.finish();
    for worktree in [&arena.first, &arena.second] {
        let record: Value = serde_json::from_slice(
            &std::fs::read(worktree.join(".gwt/skill-state/verification-run.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(record["plan_derived"], true, "{record}");
        assert_eq!(record["all_passed"], true, "{record}");
    }
    // Keep all temporary files alive through child and record inspection.
    assert!(arena.root.path().exists());
}

/// Run once against the saved baseline and once against the new binary. The
/// contender either queues behind the first matrix's parked fmt (old behavior)
/// or reaches its own Heavy command while that fmt is still active (new behavior).
/// Report measured reservation-to-admission time, never assert a speed threshold.
#[test]
#[ignore = "developer-host before/after telemetry; optionally set GWT_ADMISSION_TEST_BINARY"]
fn dev_host_reservation_to_admission_probe() {
    let arena = Arena::new();
    let commands = arena.plan(&arena.first, "a");
    assert_eq!(commands, arena.plan(&arena.second, "b"));
    let mut first = Run {
        arena: &arena,
        child: Some(arena.spawn(
            &arena.first,
            "a",
            "verify.run",
            json!({
                "commands": commands, "max_wait_secs": 30
            }),
        )),
    };
    first.wait_ready("a", "fmt");
    let mut second = Run {
        arena: &arena,
        child: Some(arena.spawn(
            &arena.second,
            "b",
            "verify.run",
            json!({
                "commands": commands, "max_wait_secs": 30
            }),
        )),
    };
    let contender_pid = second.child.as_ref().unwrap().id();
    let deadline = Instant::now() + Duration::from_secs(30);
    let queued = loop {
        let status = collect(arena.spawn(&arena.second, "b", "verify.lease.status", json!({})));
        if let Some(line) = status.lines().find(|line| line.starts_with("queue[0]:")) {
            let field = |prefix: &str| {
                line.split_whitespace()
                    .find_map(|part| part.strip_prefix(prefix))
                    .unwrap()
                    .to_owned()
            };
            break Some((
                field("target="),
                field("queued_at_ms=").parse::<u64>().unwrap(),
            ));
        }
        if arena.marker("b", "fmt", "ready").exists() {
            arena.release("b", "fmt");
            second.wait_ready("b", "clippy");
            break None;
        }
        assert!(
            Instant::now() < deadline,
            "contender neither queued nor started: {status}"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    // Release both entire matrices only after observing the admission event.
    std::fs::write(arena.signals.join("release-all"), "release").unwrap();
    first.finish();
    second.finish();
    if let Some((target, queued_at_ms)) = queued {
        let ledger = arena
            .home
            .join(".gwt/runtime/verification-coordinator/lease-events.jsonl");
        let event = std::fs::read_to_string(ledger)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|event| event["kind"] == "acquired" && event["target"] == target)
            .expect("contender acquisition event");
        let acquired_at_ms = event["at_ms"].as_u64().unwrap();
        eprintln!("matrix admission: contender_pid={contender_pid} queued_at_ms={queued_at_ms} acquired_at_ms={acquired_at_ms} queue_wait_ms={}", acquired_at_ms.saturating_sub(queued_at_ms));
    } else {
        let record: Value = serde_json::from_slice(
            &std::fs::read(arena.second.join(".gwt/skill-state/verification-run.json")).unwrap(),
        )
        .unwrap();
        let heavy = record["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| {
                command["command"]
                    .as_str()
                    .is_some_and(|command| command.starts_with("cargo clippy"))
            })
            .unwrap();
        let wait = heavy["admission"]["queue_wait_ms"]
            .as_u64()
            .expect("Heavy admission telemetry");
        eprintln!("per-command admission: contender_pid={contender_pid} queue_wait_ms={wait}; clippy admitted while first matrix fmt remained active");
    }
}
