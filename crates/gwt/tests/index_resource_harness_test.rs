//! Issue #4527 AC-2 / SPEC #1939 T-IDX-398 / T-IDX-442: real E5 resource
//! ceiling harness.
//!
//! Builds a synthetic project with the real `intfloat/multilingual-e5-base`
//! runner, samples the runner's logical process tree through
//! [`measure_process_tree`], then measures warm searches. It prints one JSON
//! report with every measured value next to its ceiling and hard-fails when a
//! ceiling is exceeded.
//!
//! `#[ignore]` because it bootstraps the runner runtime and needs the model cache:
//! `cargo test -p gwt --test runtime_tests index_resource_harness_test:: -- --ignored --nocapture`.
//! The model is read from the caller's HuggingFace cache (`HF_HOME`, else
//! `~/.cache/huggingface`); the index itself goes to a temporary HOME.
//!
//! The handover acceptance test also joins native coordinator admission to
//! real model results; sustained queue fairness stays in `index_coordinator`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::{Duration, Instant},
};

use gwt::index_resources::{measure_process_tree, ProcessTreeUsage};
use serde_json::json;

const GIB: u64 = 1024 * 1024 * 1024;
const BACKGROUND_CPU_CEILING: f64 = 220.0;
const INTERACTIVE_CPU_CEILING: f64 = 420.0;
const RSS_CEILING: u64 = 5 * GIB / 2;
const PRIVATE_CEILING: u64 = 5 * GIB;
const WARM_SEARCH_P95_CEILING_MS: u128 = 5_000;
const WARM_SEARCH_MAX_CEILING_MS: u128 = 8_000;
const SOURCE_FILES: usize = 300;
const WARM_SEARCHES: usize = 10;

#[derive(Default)]
struct Peak {
    samples: Vec<f64>,
    rss_bytes: u64,
    private_bytes: Option<u64>,
    max_processes: usize,
}

impl Peak {
    fn record(&mut self, usage: &ProcessTreeUsage) {
        self.samples.push(usage.cpu_percent);
        self.rss_bytes = self.rss_bytes.max(usage.rss_bytes);
        self.private_bytes = match (self.private_bytes, usage.private_bytes) {
            (Some(peak), Some(value)) => Some(peak.max(value)),
            (peak, value) => peak.or(value),
        };
        self.max_processes = self.max_processes.max(usage.process_count);
    }

    fn cpu_p95(&self) -> f64 {
        let mut sorted = self.samples.clone();
        sorted.sort_by(f64::total_cmp);
        percentile(&sorted).copied().unwrap_or(0.0)
    }

    fn cpu_max(&self) -> f64 {
        self.samples.iter().copied().fold(0.0, f64::max)
    }

    fn report(&self, cpu_ceiling: f64) -> serde_json::Value {
        json!({
            "samples": self.samples.len(),
            "cpu_percent_p95": self.cpu_p95(),
            "cpu_percent_max": self.cpu_max(),
            "cpu_percent_ceiling": cpu_ceiling,
            "rss_bytes_peak": self.rss_bytes,
            "rss_bytes_ceiling": RSS_CEILING,
            "private_bytes_peak": self.private_bytes,
            "private_bytes_ceiling": PRIVATE_CEILING,
            "max_processes": self.max_processes,
        })
    }

    fn violations(&self, phase: &str, cpu_ceiling: f64) -> Vec<String> {
        let mut violations = Vec::new();
        if self.samples.is_empty() {
            violations.push(format!("{phase}: no resource sample was taken"));
        }
        if self.cpu_p95() > cpu_ceiling {
            violations.push(format!(
                "{phase}: CPU p95 {:.1}% > {cpu_ceiling}%",
                self.cpu_p95()
            ));
        }
        if self.rss_bytes > RSS_CEILING {
            violations.push(format!("{phase}: RSS {} > {RSS_CEILING}", self.rss_bytes));
        }
        if self
            .private_bytes
            .is_some_and(|bytes| bytes > PRIVATE_CEILING)
        {
            violations.push(format!(
                "{phase}: private {:?} > {PRIVATE_CEILING}",
                self.private_bytes
            ));
        }
        violations
    }
}

fn percentile<T>(sorted: &[T]) -> Option<&T> {
    let rank = (sorted.len() * 95).div_ceil(100);
    sorted.get(rank.saturating_sub(1))
}

fn huggingface_cache() -> PathBuf {
    std::env::var_os("HF_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cache/huggingface")))
        .expect("resolve the HuggingFace cache")
}

fn write_project(root: &Path) {
    let topics = [
        "filesystem watcher debounce",
        "semantic search ranking",
        "terminal emulation scrollback",
        "git worktree lifecycle",
        "websocket reconnection backoff",
        "issue cache refresh",
    ];
    fs::create_dir_all(root.join("src")).unwrap();
    for index in 0..SOURCE_FILES {
        let topic = topics[index % topics.len()];
        let body: String = (0..40)
            .map(|line| {
                format!("// {topic} detail {index}-{line}\npub fn f{index}_{line}() {{}}\n")
            })
            .collect();
        fs::write(
            root.join(format!("src/module_{index}.rs")),
            format!("//! {topic} module {index}\n{body}"),
        )
        .unwrap();
    }
}

struct Harness {
    python: PathBuf,
    runner: PathBuf,
    home: PathBuf,
    hf_home: PathBuf,
    project: PathBuf,
    repo_hash: String,
    worktree_hash: String,
}

impl Harness {
    fn spawn(&self, action: &str, extra: &[&str]) -> Child {
        gwt_core::process::hidden_command(&self.python)
            .arg(&self.runner)
            .args(["--action", action, "--repo-hash", &self.repo_hash])
            .args(["--worktree-hash", &self.worktree_hash])
            .arg("--project-root")
            .arg(&self.project)
            .args(extra)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("HF_HOME", &self.hf_home)
            .env("GWT_INDEX_FAKE_EMBEDDING", "0")
            .env(
                "GWT_INDEX_COORDINATOR_ROOT",
                self.home.join(".gwt/runtime/index-coordinator"),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn runner")
    }

    /// Run to completion while sampling the runner tree.
    fn run_sampled(&self, action: &str, extra: &[&str], peak: &mut Peak) -> std::time::Duration {
        let started = Instant::now();
        let mut child = self.spawn(action, extra);
        while child.try_wait().expect("poll runner").is_none() {
            if let Some(usage) = measure_process_tree(child.id()) {
                peak.record(&usage);
            }
        }
        let elapsed = started.elapsed();
        let output = child.wait_with_output().expect("collect runner output");
        assert!(
            output.status.success(),
            "{action} exit={:?} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        elapsed
    }
}

/// A worker returned by this fixture must release its model before HOME is removed.
struct HarnessWorker(sysinfo::Pid, u64);

impl HarnessWorker {
    fn new(pid: u32) -> Self {
        let pid = sysinfo::Pid::from_u32(pid);
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        Self(
            pid,
            system
                .process(pid)
                .expect("live fixture worker")
                .start_time(),
        )
    }
}

impl Drop for HarnessWorker {
    fn drop(&mut self) {
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[self.0]), true);
        if let Some(process) = system
            .process(self.0)
            .filter(|process| process.start_time() == self.1)
        {
            let identity = self.1;
            let _ = process.kill();
            let started = Instant::now();
            loop {
                system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[self.0]), true);
                if system.process(self.0).is_none_or(|process| {
                    process.start_time() != identity
                        || matches!(
                            process.status(),
                            sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
                        )
                }) {
                    break;
                }
                if started.elapsed() >= Duration::from_secs(5) {
                    assert!(
                        std::thread::panicking(),
                        "fixture search worker did not exit"
                    );
                    eprintln!("fixture search worker {:?} did not exit", self.0);
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Issue #5437: exercise the checkout runner, real E5, and published indexes on Windows.
/// Invoked explicitly by nightly CI; missing Python/model assets must fail, never skip.
#[test]
#[ignore]
fn real_e5_all_scopes_build_and_search() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let harness = Harness {
        python: std::env::var_os("PYTHON").map_or_else(|| PathBuf::from("python"), PathBuf::from),
        runner: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../gwt-core/runtime/chroma_index_runner.py"),
        home: tmp.path().join("home"),
        hf_home: huggingface_cache(),
        repo_hash: gwt_core::repo_hash::compute_repo_hash(
            "https://github.com/example/all-scopes-e2e.git",
        )
        .to_string(),
        worktree_hash: gwt_core::worktree_hash::compute_worktree_hash(&project)
            .unwrap()
            .to_string(),
        project,
    };
    let write = |path: PathBuf, content: String| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    };
    let state = harness.home.join(".gwt/projects").join(&harness.repo_hash);
    let mut memory = String::new();
    let mut discussions = String::new();
    let mut board = String::new();
    let mut works = Vec::new();
    // Four distractors make top-three ranking meaningful rather than a nonempty check.
    for (index, topic) in [
        "filesystem watcher debounce",
        "sourdough bread baking recipe",
        "distant galaxy astronomy telescope",
        "ceramic pottery kiln glazing",
        "bicycle mountain racing training",
    ]
    .iter()
    .enumerate()
    {
        let name = if index == 0 {
            "watcher".to_string()
        } else {
            format!("unrelated_{index}")
        };
        write(
            harness.project.join(format!("src/{name}.rs")),
            format!("//! {topic}\npub fn {name}() {{}}\n"),
        );
        write(
            harness.project.join(format!("docs/{name}.md")),
            format!("# {topic}\n\nDocumentation about {topic}.\n"),
        );
        for (offset, labels) in [(100, json!(["bug"])), (200, json!(["gwt-spec"]))] {
            let number = offset + index;
            let cache = harness
                .home
                .join(".gwt/cache/issues")
                .join(&harness.repo_hash)
                .join(number.to_string());
            write(
                cache.join("meta.json"),
                json!({"number": number, "title": topic, "labels": labels,
                    "state": "OPEN", "updated_at": "2026-10-15T00:00:00Z"})
                .to_string(),
            );
            write(cache.join("body.md"), format!("# {topic}\n\n{topic}\n"));
            if offset == 200 {
                write(
                    cache.join("sections/spec.md"),
                    format!("# {topic}\n\n{topic}\n"),
                );
            }
        }
        memory.push_str(&format!(
            "## 2026-10-15 — {topic}\n\nLearning: {topic}.\n\n"
        ));
        discussions.push_str(&format!(
            "## 2026-10-15 — {topic}\n\nStatus: active\nTopics: {topic}\n\nSummary:\n{topic}\n\n"
        ));
        board.push_str(&format!(
            "{}\n",
            json!({"type": "message_appended", "entry": {
                "id": name, "kind": "decision", "author": "Fixture",
                "title_summary": topic, "body": topic,
                "created_at": "2026-10-15T00:00:00Z", "updated_at": "2026-10-15T00:00:00Z"
            }})
        ));
        works.push(json!({"id": name, "title": topic, "intent": topic,
            "summary": topic, "status_category": "done", "events": []}));
    }
    write(state.join("work-notes/memory.md"), memory);
    write(state.join("work-notes/discussions.md"), discussions);
    write(state.join("coordination/events/000001.jsonl"), board);
    write(
        state.join("coordination/events.manifest.json"),
        json!({"version": 1, "segments": [{"file": "000001.jsonl"}]}).to_string(),
    );
    write(
        state.join("project-state/works.json"),
        json!({"work_items": works}).to_string(),
    );
    let invoke = |action: &str, args: &[&str]| {
        let output = harness.spawn(action, args).wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{action}: {} {}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("runner JSON");
        assert_eq!(value["ok"], true, "{action}: {value}");
        value
    };
    let mut workers = Vec::new();
    for (scope, key, identity, expected) in [
        ("specs", "specResults", "spec_id", json!("200")),
        ("issues", "issueResults", "number", json!(100)),
        ("files", "results", "path", json!("src/watcher.rs")),
        ("files-docs", "results", "path", json!("docs/watcher.md")),
        (
            "memory",
            "memoryResults",
            "title",
            json!("filesystem watcher debounce"),
        ),
        ("board", "boardResults", "entry_id", json!("watcher")),
        (
            "discussions",
            "discussionResults",
            "title",
            json!("filesystem watcher debounce"),
        ),
        ("works", "workResults", "work_id", json!("watcher")),
    ] {
        let started = Instant::now();
        let build = invoke(
            &format!("index-{scope}"),
            &[
                "--scope",
                scope,
                "--file-index-protocol",
                "v2",
                "--qos",
                "interactive",
            ],
        );
        let build_ms = started.elapsed().as_millis();
        assert_ne!(build["yielded"], true, "{scope}: {build}");
        let args = [
            "--scopes",
            scope,
            "--query",
            "filesystem watcher debounce",
            "--n-results",
            "3",
            "--no-auto-build",
            "--file-index-protocol",
            "v2",
        ];
        let first_started = Instant::now();
        let first = invoke("search-multi", &args);
        let first_search_ms = first_started.elapsed().as_millis();
        workers.push(HarnessWorker::new(
            first["worker_pid"].as_u64().expect("real worker") as u32,
        ));
        let warm_started = Instant::now();
        let result = invoke("search-multi", &args);
        let warm_ms = warm_started.elapsed().as_millis();
        let hits = result["scope_results"][scope][key]
            .as_array()
            .expect("scope results");
        assert!(
            hits.iter().any(|hit| hit[identity] == expected),
            "{scope}: {result}"
        );
        assert_eq!(
            result["model_load_ms"], 0,
            "{scope}: warm query reloaded model"
        );
        assert!(
            warm_ms <= 5_000,
            "{scope}: warm search took {warm_ms}ms > 5000ms"
        );
        println!(
            "all-scope E5 report: {}",
            json!({
                "platform": std::env::consts::OS, "scope": scope, "build_ms": build_ms,
                "first_search_ms": first_search_ms, "first_model_load_ms": first["model_load_ms"],
                "warm_ms": warm_ms, "warm_ceiling_ms": 5000,
                "top_three": hits, "build": build,
            })
        );
    }
    drop(workers); // Close only fixture-owned workers before removing their HOME/indexes.
}

/// Issue #5431: warm means both the query worker and this background model are ready.
/// Requires the query worker delivered by Issue #5434; no cold-load time is subtracted.
/// Set HF_HOME to the prepared model cache; PYTHON may select a prepared interpreter.
#[test]
#[ignore]
fn real_warm_search_finishes_during_background_build_and_build_resumes() {
    use gwt_core::index_coordinator::{
        IndexCoordinator, JobAdmission, JobOutcome, JobPriority, TargetKey,
        INTERACTIVE_SEARCH_ADMISSION_DEADLINE,
    };
    let hf_home = std::env::var_os("HF_HOME")
        .map(PathBuf::from)
        .expect("HF_HOME must name the prepared E5 model cache");
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    fs::create_dir_all(project.join("src")).unwrap();
    for index in 0..24 {
        let topic = if index == 0 {
            "filesystem watcher debounce"
        } else {
            "terminal scrollback renderer"
        };
        fs::write(
            project.join(format!("src/module_{index}.rs")),
            format!("//! {topic} module {index}\npub fn feature_{index}() {{}}\n"),
        )
        .unwrap();
    }
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let runner = tmp.path().join("chroma_index_runner.py");
    fs::write(
        &runner,
        include_str!("../../gwt-core/runtime/chroma_index_runner.py"),
    )
    .unwrap();
    fs::write(
        tmp.path().join("index_path_policy.json"),
        include_str!("../../gwt-core/runtime/index_path_policy.json"),
    )
    .unwrap();
    let harness = Harness {
        python: std::env::var_os("PYTHON").map_or_else(|| PathBuf::from("python"), PathBuf::from),
        runner,
        repo_hash: gwt_core::repo_hash::compute_repo_hash(
            "https://github.com/example/search-handover.git",
        )
        .to_string(),
        worktree_hash: gwt_core::worktree_hash::compute_worktree_hash(&project)
            .unwrap()
            .to_string(),
        project,
        home,
        hf_home,
    };
    let coordinator =
        IndexCoordinator::open(harness.home.join(".gwt/runtime/index-coordinator")).unwrap();
    let key = TargetKey::worktree(&harness.repo_hash, "files", &harness.worktree_hash);
    let request_owner = |priority| match coordinator
        .request_job(&key, priority, Duration::from_secs(30))
        .unwrap()
    {
        JobAdmission::Owner(guard) => guard,
        JobAdmission::Joined(_) => panic!("isolated fixture must own its build"),
    };
    let payload = |output: std::process::Output| {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("runner JSON");
        assert_eq!(value["ok"], true, "{value}");
        value
    };
    let invoke =
        |action, args: &[&str]| payload(harness.spawn(action, args).wait_with_output().unwrap());
    let build_args: Vec<_> = "--scope files --mode full --file-index-protocol v2"
        .split_whitespace()
        .collect();
    let search_args = [
        "--scopes",
        "files",
        "--query",
        "filesystem watcher debounce",
        "--file-index-protocol",
        "v2",
        "--no-auto-build",
    ];
    let search_key = TargetKey::search(&harness.repo_hash, Some(&harness.worktree_hash));
    let baseline = request_owner(JobPriority::ManualRebuild);
    let heavy = baseline.acquire_heavy(Duration::from_secs(30)).unwrap();
    invoke("index-files", &build_args);
    drop(heavy);
    baseline.complete(JobOutcome::Completed).unwrap();
    let lease = coordinator
        .acquire_interactive_search_heavy(&search_key, INTERACTIVE_SEARCH_ADMISSION_DEADLINE)
        .unwrap();
    let cold_started = Instant::now();
    let first = invoke("search-multi", &search_args);
    let cold_query_ms = cold_started.elapsed().as_millis();
    let worker_pid = first["worker_pid"]
        .as_u64()
        .expect("Issue #5434 resident worker") as u32;
    let _worker = HarnessWorker::new(worker_pid);
    assert!(
        first["model_load_ms"].as_u64().is_some_and(|ms| ms > 0),
        "{first}"
    );
    drop(lease);
    for index in 0..24 {
        fs::write(harness.project.join(format!("src/module_{index}.rs")), format!("//! revised filesystem watcher debounce {index}\npub fn updated_watcher_{index}() {{}}\n")).unwrap();
    }
    let guard = request_owner(JobPriority::Background);
    let heavy = guard.acquire_heavy(Duration::from_secs(30)).unwrap();
    let lease_id = heavy.id().to_string();
    let background_args: Vec<_> = build_args
        .iter()
        .copied()
        .chain(["--qos", "background"])
        .collect();
    let child = harness.spawn("index-files", &background_args);
    let warming_started = Instant::now();
    let (background, lease_wait_ms, warm_search_ms, background_warming_ms) =
        std::thread::scope(|scope| {
            let background = scope.spawn(move || {
                // wait_with_output drains both pipes, then reaps before releasing exclusion.
                let output = child.wait_with_output().unwrap();
                drop(heavy);
                output
            });
            let mut saw_warming = false;
            loop {
                let status = coordinator.heavy_lease_status().unwrap();
                saw_warming |= status.model_state.as_deref() == Some("warming");
                assert!(
                    !background.is_finished(),
                    "background ended before warm contention"
                );
                assert_eq!(status.lease_id.as_deref(), Some(lease_id.as_str()));
                assert!(
                    warming_started.elapsed() < Duration::from_secs(30),
                    "background warming exceeded 30s: {status:?}"
                );
                if status.model_state.as_deref() == Some("warm") {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(
                saw_warming,
                "background must expose its cold initialization"
            );
            let background_warming_ms = warming_started.elapsed().as_millis();
            let started = Instant::now();
            let lease = coordinator
                .acquire_interactive_search_heavy(
                    &search_key,
                    INTERACTIVE_SEARCH_ADMISSION_DEADLINE,
                )
                .unwrap();
            let lease_wait_ms = started.elapsed().as_millis();
            let result = invoke("search-multi", &search_args);
            let warm_search_ms = started.elapsed().as_millis();
            drop(lease);
            assert_eq!(result["worker_pid"].as_u64(), Some(u64::from(worker_pid)));
            assert_eq!(result["model_load_ms"], 0, "{result}");
            assert!(
                result["scope_results"]["files"]["results"]
                    .as_array()
                    .expect("file results")
                    .iter()
                    .any(|hit| hit["path"] == "src/module_0.rs"),
                "{result}"
            );
            (
                payload(background.join().unwrap()),
                lease_wait_ms,
                warm_search_ms,
                background_warming_ms,
            )
        });
    assert_eq!(
        background["yielded"], true,
        "search must actually preempt the background: {background}"
    );
    let computed = background["computed_embeddings"].as_u64().unwrap();
    assert!(computed > 0 && computed < 24, "{background}");
    let heavy = guard.acquire_heavy(Duration::from_secs(30)).unwrap();
    let resumed = invoke("index-files", &background_args);
    drop(heavy);
    guard.complete(JobOutcome::Completed).unwrap();
    assert_ne!(resumed["yielded"], true, "{resumed}");
    assert_eq!(resumed["computed_embeddings"].as_u64(), Some(24 - computed));
    assert_eq!(resumed["embedding_cache_hits"].as_u64(), Some(computed));
    println!(
        "search handover report: {}",
        json!({"platform": std::env::consts::OS, "cold_query_ms": cold_query_ms, "background_warming_ms": background_warming_ms, "lease_wait_ms": lease_wait_ms, "warm_search_ms": warm_search_ms, "background_computed": computed, "resumed": resumed})
    );
    assert!(
        cold_query_ms <= 30_000,
        "cold query exceeded 30s: {cold_query_ms}ms"
    );
    assert!(
        warm_search_ms <= 5_000,
        "warm lease plus search exceeded 5s: {warm_search_ms}ms"
    );
}

#[test]
#[ignore]
fn real_e5_runner_stays_within_resource_ceilings() {
    gwt_core::runtime::ensure_project_index_runtime().expect("bootstrap the index runtime");
    let python = gwt_core::runtime::project_index_python_path();
    let runner = gwt_core::runtime::project_index_runner_path();
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    write_project(&project);
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let harness = Harness {
        python,
        runner,
        home,
        hf_home: huggingface_cache(),
        repo_hash: gwt_core::repo_hash::compute_repo_hash(
            "https://github.com/example/resource-harness.git",
        )
        .as_str()
        .to_string(),
        worktree_hash: gwt_core::worktree_hash::compute_worktree_hash(&project)
            .unwrap()
            .as_str()
            .to_string(),
        project,
    };

    let mut build = Peak::default();
    let build_elapsed = harness.run_sampled(
        "index-files",
        &["--scope", "files", "--mode", "full", "--qos", "background"],
        &mut build,
    );

    let mut search = Peak::default();
    let mut latencies: Vec<u128> = (0..WARM_SEARCHES)
        .map(|_| {
            harness
                .run_sampled(
                    "search-files",
                    &[
                        "--query",
                        "watcher debounce",
                        "--n-results",
                        "5",
                        "--no-auto-build",
                    ],
                    &mut search,
                )
                .as_millis()
        })
        .collect();
    latencies.sort_unstable();
    let search_p95 = *percentile(&latencies).unwrap();
    let search_max = *latencies.last().unwrap();

    let report = json!({
        "platform": std::env::consts::OS,
        "source_files": SOURCE_FILES,
        "build": {
            "elapsed_ms": build_elapsed.as_millis(),
            "resources": build.report(BACKGROUND_CPU_CEILING),
        },
        "warm_search": {
            "runs": WARM_SEARCHES,
            "latency_ms": latencies,
            "p95_ms": search_p95,
            "p95_ceiling_ms": WARM_SEARCH_P95_CEILING_MS,
            "max_ms": search_max,
            "max_ceiling_ms": WARM_SEARCH_MAX_CEILING_MS,
            "resources": search.report(INTERACTIVE_CPU_CEILING),
        },
    });
    println!(
        "index resource harness report: {}",
        serde_json::to_string_pretty(&report).unwrap()
    );

    let mut violations = build.violations("background build", BACKGROUND_CPU_CEILING);
    violations.extend(search.violations("interactive search", INTERACTIVE_CPU_CEILING));
    if search_p95 > WARM_SEARCH_P95_CEILING_MS {
        violations.push(format!(
            "warm search p95 {search_p95}ms > {WARM_SEARCH_P95_CEILING_MS}ms"
        ));
    }
    if search_max > WARM_SEARCH_MAX_CEILING_MS {
        violations.push(format!(
            "warm search max {search_max}ms > {WARM_SEARCH_MAX_CEILING_MS}ms"
        ));
    }
    assert!(
        violations.is_empty(),
        "resource ceilings exceeded: {violations:#?}"
    );
}
