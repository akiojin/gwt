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
//!
//! Issue #4134 extends the same subject from one step's ceiling to the
//! workflow's critical path. `test-windows-default-parallel` measured 2806s —
//! `test.yml`'s whole critical path — re-running on Windows a suite Linux had
//! already run, and the agent-launch matrix paid the same 279s cold build four
//! times to run 137s of tests. Those are composition defects, so the
//! composition is pinned here alongside the budgets.

use std::fs;
use std::path::PathBuf;

const TEST_WORKFLOW: &str = ".github/workflows/test.yml";
const NIGHTLY_WORKFLOW: &str = ".github/workflows/nightly.yml";
const RUST_JOB: &str = "  test:\n";
const RUN_TESTS_STEP: &str = "Run tests";
const DEFAULT_PARALLEL_JOB: &str = "  test-windows-default-parallel:";
const AGENT_LAUNCH_JOB: &str = "  test-windows-agent-launch-e2e:";
/// The four provider/selector combinations the deterministic Windows launch
/// E2E has to cover, previously one matrix shard each.
const AGENT_LAUNCH_COMBINATIONS: [&str; 4] = [
    "codex/latest",
    "codex/exact",
    "claude/latest",
    "claude/exact",
];

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

/// The job body from its header up to the next top-level job.
fn job_body<'a>(workflow: &'a str, header: &str) -> &'a str {
    let start = workflow
        .find(header)
        .unwrap_or_else(|| panic!("workflow must define `{}`", header.trim()));
    let body = &workflow[start + header.len()..];
    let end = body.find("\n\n  ").map(|at| at + 1).unwrap_or(body.len());
    &body[..end]
}

/// Issue #4134 AC-1: the three-pass determinism loop is the single most
/// expensive thing PR CI used to do, and it re-ran a suite Linux had already
/// proven. It keeps its purpose on a nightly schedule; PR CI must not pay for
/// it.
#[test]
fn the_three_pass_determinism_loop_left_pr_ci_for_the_nightly_schedule() {
    let workflow = read(TEST_WORKFLOW);
    assert!(
        !workflow.contains(DEFAULT_PARALLEL_JOB),
        "{TEST_WORKFLOW} runs on pull_request only, so it must not host \
         `test-windows-default-parallel` (Issue #4134 AC-1)"
    );
    assert!(
        !workflow.contains("1..3 | ForEach-Object"),
        "the three-pass loop measured 2806s and was {TEST_WORKFLOW}'s critical \
         path; it must not run per pull request"
    );

    let nightly = read(NIGHTLY_WORKFLOW);
    assert!(
        !nightly.contains("pull_request"),
        "{NIGHTLY_WORKFLOW} exists to keep scheduled work off the PR path"
    );
    assert!(
        nightly.contains("schedule:") && nightly.contains("cron:"),
        "{NIGHTLY_WORKFLOW} must declare the schedule that replaces the PR run"
    );
    let job = job_body(&nightly, DEFAULT_PARALLEL_JOB);
    assert!(
        job.contains("runs-on: windows-latest"),
        "the determinism proof still has to run on a native Windows runner"
    );
    let build_at = job
        .find("--all-features --no-run")
        .expect("the determinism job must build outside the timed loop");
    let loop_at = job
        .find("1..3 | ForEach-Object")
        .expect("the determinism job must keep its three-pass loop");
    assert!(
        build_at < loop_at,
        "the --no-run build must precede the loop, or a slow compile reads as \
         an unstable suite"
    );
}

/// Issue #4134 AC-1: a nightly job nobody watches is a job nobody runs, so the
/// schedule only replaces PR CI if a failure reaches a named destination.
#[test]
fn nightly_determinism_failures_have_an_explicit_notification_target() {
    let nightly = read(NIGHTLY_WORKFLOW);
    assert!(
        nightly.contains("if: failure()"),
        "{NIGHTLY_WORKFLOW} must react to a failed determinism run"
    );
    assert!(
        nightly.contains("gh issue create"),
        "{NIGHTLY_WORKFLOW} must name where a nightly failure is reported; a \
         scheduled run has no PR to turn red"
    );
    assert!(
        nightly.contains("issues: write"),
        "the failure reporter needs the permission it uses"
    );
}

/// Issue #4134 AC-2: four matrix shards each paid a 279s cold build to run
/// 137s of tests. One job builds once and runs every combination, and a
/// failure still says which combination failed.
#[test]
fn windows_agent_launch_e2e_builds_once_and_runs_every_combination() {
    let workflow = read(TEST_WORKFLOW);
    let job = job_body(&workflow, AGENT_LAUNCH_JOB);
    assert!(
        !job.contains("matrix:"),
        "the agent-launch combinations share one cold build now, so the job \
         must not fan out over a matrix (Issue #4134 AC-2)"
    );
    let build_at = job
        .find("cargo test -p gwt --test windows_agent_launch_e2e --no-run")
        .expect("the agent-launch job must build the E2E binary once, on its own step");
    let run_at = job
        .find("cargo test -p gwt --test windows_agent_launch_e2e -- --ignored")
        .expect("the agent-launch job must still run the deterministic E2E");
    assert!(
        build_at < run_at,
        "the shared build must precede the combination loop"
    );
    for combination in AGENT_LAUNCH_COMBINATIONS {
        assert!(
            job.contains(combination),
            "the single agent-launch job must still cover {combination}"
        );
    }
    assert!(
        job.contains("::error::"),
        "collapsing the matrix must not cost per-combination attribution; a \
         failing combination has to annotate itself"
    );
    assert!(
        job.contains("status=1"),
        "the loop must keep the matrix's fail-fast: false semantics and run \
         every combination before failing"
    );
}
