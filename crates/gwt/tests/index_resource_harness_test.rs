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
//! `cargo test -p gwt --test index_resource_harness_test -- --ignored --nocapture`.
//! The model is read from the caller's HuggingFace cache (`HF_HOME`, else
//! `~/.cache/huggingface`); the index itself goes to a temporary HOME.
//!
//! Heavy-owner exclusivity and foreground acquisition are coordinator
//! properties and stay covered by the `index_coordinator` tests; this harness
//! measures what only a real model can show.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::Instant,
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
