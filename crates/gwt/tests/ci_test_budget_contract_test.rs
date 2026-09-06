//! Contract tests for the `Test (Rust)` wall-clock budget (Issue #4055).
//!
//! `cargo test --workspace --all-features` ran under `timeout-minutes: 15`
//! while the step itself measured 500–725s across 21 green runs and crossed
//! 900s on a slow runner (run 34063577896: compile ~3m, gwt lib 231s, gwt bin
//! 318s — 2.5x the usual 100s / 90s — then killed with every `test result:`
//! line `ok`). A green suite that dies on the step clock is a budget defect,
//! not a test failure, so the budget is pinned here: the step gets at least
//! +50% over the measured p95 (~11 min), and the job declares its own ceiling
//! that covers every step it runs.

use std::fs;
use std::path::PathBuf;

const TEST_WORKFLOW: &str = ".github/workflows/test.yml";
const RUST_JOB: &str = "  test:\n";
const RUN_TESTS_STEP: &str = "Run tests";

/// Measured p95 of the `Run tests` step (21 green develop-bound runs on
/// 2026-09-06) is ~660s; +50% rounds to 17 minutes, and the Issue asks for
/// a budget that also absorbs a 2.5x-slow runner, hence 25.
const MIN_RUN_TESTS_MINUTES: u64 = 25;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("gwt crate must be nested under crates/")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The `test:` job body: from its header up to the next top-level job.
fn rust_job(workflow: &str) -> &str {
    let start = workflow
        .find(RUST_JOB)
        .unwrap_or_else(|| panic!("{TEST_WORKFLOW} must define the `test` job"));
    let body = &workflow[start + RUST_JOB.len()..];
    let end = body.find("\n\n  ").map(|at| at + 1).unwrap_or(body.len());
    &body[..end]
}

fn named_steps(job: &str) -> Vec<(String, String)> {
    let mut steps = Vec::new();
    let mut rest = job;
    while let Some(at) = rest.find("\n      - name:") {
        rest = &rest[at + 1..];
        let line_end = rest.find('\n').unwrap_or(rest.len());
        let name = rest["      - name:".len()..line_end].trim().to_string();
        let body = rest[line_end..]
            .split("\n      - ")
            .next()
            .expect("split always yields a first segment")
            .to_string();
        steps.push((name, body));
    }
    steps
}

fn timeout_minutes(body: &str) -> Option<u64> {
    body.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("timeout-minutes:"))
        .and_then(|value| value.trim().parse().ok())
}

/// Job-level keys sit at six spaces directly under `test:`; step keys are
/// nested under `steps:`, so only the prefix before `steps:` is inspected.
fn job_timeout_minutes(job: &str) -> Option<u64> {
    let header = job.split("    steps:").next().unwrap_or(job);
    timeout_minutes(header)
}

/// AC-1: the `Run tests` step budget clears the measured p95 by at least
/// 50%, so a slow runner or a cold cache cannot fail a green suite.
#[test]
fn run_tests_step_budget_clears_measured_p95_with_margin() {
    let workflow = read(TEST_WORKFLOW);
    let job = rust_job(&workflow);
    let (_, body) = named_steps(job)
        .into_iter()
        .find(|(name, _)| name == RUN_TESTS_STEP)
        .unwrap_or_else(|| {
            panic!("{TEST_WORKFLOW} `test` job must keep a `{RUN_TESTS_STEP}` step")
        });
    let minutes = timeout_minutes(&body)
        .unwrap_or_else(|| panic!("`{RUN_TESTS_STEP}` must declare timeout-minutes:\n{body}"));
    assert!(
        minutes >= MIN_RUN_TESTS_MINUTES,
        "`{RUN_TESTS_STEP}` timeout-minutes is {minutes}; the step measured \
         500–725s green and exceeded 900s on a slow runner, so it needs at \
         least {MIN_RUN_TESTS_MINUTES} minutes (Issue #4055)"
    );
}

/// AC-1: the job ceiling is explicit and covers every step budget it runs,
/// so the step budgets stay the operative limits instead of an implicit
/// 360-minute job default or a job cap tighter than its own steps.
#[test]
fn rust_job_ceiling_is_explicit_and_covers_its_step_budgets() {
    let workflow = read(TEST_WORKFLOW);
    let job = rust_job(&workflow);
    let job_minutes = job_timeout_minutes(job)
        .unwrap_or_else(|| panic!("`test` job must declare its own timeout-minutes:\n{job}"));
    let step_total: u64 = named_steps(job)
        .iter()
        .filter_map(|(_, body)| timeout_minutes(body))
        .sum();
    assert!(
        job_minutes >= step_total,
        "`test` job timeout-minutes {job_minutes} is below the sum of its \
         step budgets {step_total}; the job cap would preempt the step caps"
    );
}
