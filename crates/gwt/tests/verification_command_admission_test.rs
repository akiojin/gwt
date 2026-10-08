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
    if command == "metadata" {
        assert_eq!(env::args().skip(1).collect::<Vec<_>>(),
            ["metadata", "--offline", "--no-deps", "--format-version", "1"]);
        println!("{}", env::var("ADMISSION_TARGET_METADATA").unwrap());
        return;
    }
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
        std::fs::create_dir_all(home.join(".gwt")).unwrap();
        std::fs::write(
            home.join(".gwt/config.toml"),
            "[verification]\nslots=1\ndisk_budget_bytes=0\n[build_artifact_gc]\nbelow_bytes=0\nbelow_percent=0\n",
        )
        .unwrap();
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
            .env(
                "ADMISSION_TARGET_METADATA",
                json!({"target_directory": worktree.join("target")}).to_string(),
            )
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

/// PM ruling for #5082: four real canonical matrices, with identical warmed
/// independent targets. This is a developer-host measurement, not a CI timing
/// assertion or a substitute for the final worktree's verification matrix.
#[test]
#[ignore = "real Cargo throughput and disk measurement; preserves machine-local evidence"]
fn canonical_four_matrix_throughput_probe() {
    use std::sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    };

    #[cfg(windows)]
    const MATRIX: [&str; 3] = [
        "cargo clippy -p gwt-core -p gwt-config --all-targets -- -D warnings",
        "cargo nextest run --profile gwt-verify -p gwt-core --all-features --retries 0",
        "cargo nextest run --profile gwt-verify -p gwt-config --lib",
    ];
    #[cfg(not(windows))]
    const MATRIX: [&str; 3] = [
        "cargo clippy -p gwt-core -p gwt-config --all-targets -- -D warnings",
        "cargo test -p gwt-core --all-features",
        "cargo test -p gwt-config --lib",
    ];

    fn bytes(path: &Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(path) else {
            return 0;
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| match entry.file_type() {
                Ok(kind) if kind.is_dir() => bytes(&entry.path()),
                Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |meta| meta.len()),
                _ => 0,
            })
            .sum()
    }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().map(Result::unwrap) {
            let destination = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &destination);
            } else {
                // Independent files, so Cargo cannot mutate another target.
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
    }

    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = tempfile::Builder::new()
        .prefix("gwt-5082-canonical-")
        .tempdir()
        .unwrap()
        .keep();
    eprintln!("canonical benchmark evidence: {}", root.display());
    let home = root.join("home");
    std::fs::create_dir_all(home.join(".gwt")).unwrap();
    let config = home.join(".gwt/config.toml");
    let set_slots = |slots| {
        std::fs::write(&config, format!("[verification]\nslots = {slots}\n")).unwrap();
    };
    set_slots(1);
    let tracked = hidden_command("git")
        .current_dir(&source)
        .args(["ls-files", "-z"])
        .output()
        .unwrap();
    assert!(tracked.status.success());
    let files: Vec<_> = tracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| std::str::from_utf8(path).unwrap())
        .filter(|path| !path.starts_with(".gwt/work/events"))
        .collect();
    let workers: Vec<_> = (0..4)
        .map(|index| root.join(format!("worker-{index}")))
        .collect();
    // Snapshot once before creating the four test-only repositories. These
    // fixtures never create or switch the agent's managed worktree/branches.
    let snapshot = root.join("source-snapshot");
    std::fs::create_dir_all(&snapshot).unwrap();
    for relative in files {
        let from = source.join(relative);
        if from.is_file() {
            let to = snapshot.join(relative);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(from, to).unwrap();
        }
    }
    for (index, worker) in workers.iter().enumerate() {
        copy_tree(&snapshot, worker);
        git(worker, &["init", "-q", "-b", "benchmark"]);
        git(worker, &["config", "user.email", "test@example.com"]);
        git(worker, &["config", "user.name", "Test"]);
        git(worker, &["config", "core.autocrlf", "false"]);
        git(worker, &["add", "."]);
        git(worker, &["commit", "-qm", "benchmark source snapshot"]);
        git(
            worker,
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
        );
        // Non-repository temp fixtures must not discover the worker's Git root.
        std::fs::create_dir_all(root.join(format!("tmp-{index}"))).unwrap();
    }

    let binary = std::env::var_os("GWT_ADMISSION_TEST_BINARY")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_gwtd").into());
    let cargo_home = std::env::var_os("CARGO_HOME").unwrap_or_else(|| {
        dirs::home_dir()
            .expect("original Cargo home")
            .join(".cargo")
            .into_os_string()
    });
    let rustup_home = std::env::var_os("RUSTUP_HOME").unwrap_or_else(|| {
        dirs::home_dir()
            .expect("original Rust toolchain home")
            .join(".rustup")
            .into_os_string()
    });
    let spawn = |index: usize, phase: &str, operation: &str| {
        let worker = &workers[index];
        let temporary = root.join(format!("tmp-{index}"));
        let mut command = hidden_command(&binary);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GWT_") {
                command.env_remove(key);
            }
        }
        let stdout =
            std::fs::File::create(root.join(format!("{phase}-{index}-{operation}.log"))).unwrap();
        let stderr = stdout.try_clone().unwrap();
        let mut child = command
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("GWT_HOME", home.join(".gwt"))
            .env(
                "GWT_SESSION_ID",
                format!("canonical-benchmark-{phase}-{index}"),
            )
            .env("GWT_VERIFY_SPAWN_HOST", "inherit")
            .env("CARGO_HOME", &cargo_home)
            .env("RUSTUP_HOME", &rustup_home)
            .env("CARGO_BUILD_JOBS", "1")
            .env("CARGO_TARGET_DIR", worker.join("target"))
            .env("TMPDIR", &temporary)
            .env("TEMP", &temporary)
            .env("TMP", &temporary)
            .current_dir(worker)
            .stdin(Stdio::piped())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .unwrap();
        let params = if operation == "verify.plan" {
            json!({"commands": MATRIX})
        } else {
            json!({"commands": MATRIX, "max_wait_secs": 600})
        };
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                json!({"schema_version": 1, "operation": operation, "params": params})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        child
    };
    let run = |index: usize, phase: &str| {
        assert!(spawn(index, phase, "verify.plan").wait().unwrap().success());
        spawn(index, phase, "verify.run")
    };
    let finish = |mut child: Child, index: usize, phase: &str| {
        let status = child.wait().unwrap();
        let record: Value = serde_json::from_slice(
            &std::fs::read(workers[index].join(".gwt/skill-state/verification-run.json")).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join(format!("{phase}-{index}-record.json")),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();
        assert!(
            status.success(),
            "{phase} worker {index}: see {}",
            root.display()
        );
        assert_eq!(
            record["all_passed"], true,
            "{phase} worker {index}: {record}"
        );
        assert_eq!(record["commands"].as_array().unwrap().len(), MATRIX.len());
        record
    };

    // Cold growth is measured independently of the warmed speed comparison.
    let measuring = Arc::new(AtomicBool::new(true));
    let tmp_peak = Arc::new(AtomicU64::new(0));
    let target_peak = Arc::new(AtomicU64::new(0));
    let target_before_bytes = bytes(&workers[0].join("target"));
    let temporary = root.join("tmp-0");
    let tmp_before_bytes = bytes(&temporary);
    let poll = {
        let measuring = measuring.clone();
        let tmp_peak = tmp_peak.clone();
        let target_peak = target_peak.clone();
        let worker = workers[0].clone();
        let temporary = temporary.clone();
        std::thread::spawn(move || {
            let mut next_target_sample = Instant::now();
            while measuring.load(Ordering::Relaxed) {
                tmp_peak.fetch_max(bytes(&temporary), Ordering::Relaxed);
                if Instant::now() >= next_target_sample {
                    target_peak.fetch_max(bytes(&worker.join("target")), Ordering::Relaxed);
                    next_target_sample = Instant::now() + Duration::from_secs(5);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };
    let cold_started = Instant::now();
    finish(run(0, "cold"), 0, "cold");
    measuring.store(false, Ordering::Relaxed);
    poll.join().unwrap();
    target_peak.fetch_max(bytes(&workers[0].join("target")), Ordering::Relaxed);
    let target_bytes = target_peak
        .load(Ordering::Relaxed)
        .saturating_sub(target_before_bytes);
    let tmp_bytes = tmp_peak
        .load(Ordering::Relaxed)
        .saturating_sub(tmp_before_bytes);
    let budget_bytes = target_bytes
        .saturating_add(tmp_bytes)
        .saturating_mul(12)
        .div_ceil(10);
    let disk = json!({
        "matrix": MATRIX,
        "cold_elapsed_ms": cold_started.elapsed().as_millis(),
        "target_before_bytes": target_before_bytes,
        "tmp_before_bytes": tmp_before_bytes,
        "target_peak_bytes": target_bytes,
        "tmp_peak_bytes": tmp_bytes,
        "tmp_sample_ms": 100,
        "target_sample_ms": 5000,
        "proposed_disk_budget_bytes": budget_bytes,
        "target_path": workers[0].join("target"),
        "tmp_path": temporary,
        "target_volume_root": root.ancestors().last().unwrap(),
        "tmp_volume_root": root.ancestors().last().unwrap(),
        "volume_total_bytes": fs2::total_space(&root).unwrap(),
        "measurement_limit": "sampled peaks; representative changed-package matrix, not whole workspace"
    });
    std::fs::write(
        root.join("disk-measurement.json"),
        serde_json::to_vec_pretty(&disk).unwrap(),
    )
    .unwrap();
    eprintln!("cold disk measurement: {disk}");
    // Reuse dependency downloads/builds by copying bytes, then warm every
    // independent target with the same actual canonical matrix before timing.
    for worker in workers.iter().skip(1) {
        copy_tree(&workers[0].join("target"), &worker.join("target"));
    }
    set_slots(4);
    let warming: Vec<_> = (0..4).map(|index| run(index, "warm")).collect();
    for (index, child) in warming.into_iter().enumerate() {
        finish(child, index, "warm");
    }
    set_slots(1);
    let serial_started = Instant::now();
    let mut serial = Vec::new();
    for index in 0..4 {
        serial.push(finish(run(index, "serial"), index, "serial"));
    }
    let serial_ms = serial_started.elapsed().as_millis();
    set_slots(4);
    let ledger = home.join(".gwt/runtime/verification-coordinator/lease-events.jsonl");
    let parallel_ledger_start = std::fs::metadata(&ledger).unwrap().len() as usize;
    let parallel_started = Instant::now();
    let children: Vec<_> = (0..4).map(|index| run(index, "parallel")).collect();
    let parallel: Vec<_> = children
        .into_iter()
        .enumerate()
        .map(|(index, child)| finish(child, index, "parallel"))
        .collect();
    let parallel_ms = parallel_started.elapsed().as_millis();
    let events = std::fs::read_to_string(ledger).unwrap();
    let mut active = std::collections::BTreeSet::new();
    let mut maximum_parallel_holders = 0;
    for event in events[parallel_ledger_start..]
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
    {
        let lease = event["lease_id"].as_str().unwrap();
        match event["kind"].as_str().unwrap() {
            "acquired" => {
                active.insert(lease.to_owned());
                maximum_parallel_holders = maximum_parallel_holders.max(active.len());
            }
            "released" | "expired" | "reclaimed" => {
                active.remove(lease);
            }
            _ => {}
        }
    }
    let summary = json!({
        "matrix": MATRIX,
        "completed_serial": serial.len(),
        "completed_parallel": parallel.len(),
        "all_passed_serial": serial.iter().all(|record| record["all_passed"] == true),
        "all_passed_parallel": parallel.iter().all(|record| record["all_passed"] == true),
        "serial_slots": 1,
        "parallel_slots": 4,
        "maximum_parallel_holders": maximum_parallel_holders,
        "serial_elapsed_ms": serial_ms,
        "parallel_elapsed_ms": parallel_ms,
        "parallel_to_serial_ratio": parallel_ms as f64 / serial_ms as f64,
        "one_third_target_met": parallel_ms.saturating_mul(3) <= serial_ms,
        "disk_measurement": disk,
    });
    std::fs::write(
        root.join("summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    eprintln!("canonical throughput measurement: {summary}");
}
