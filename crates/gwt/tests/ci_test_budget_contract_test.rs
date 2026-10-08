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

#[test]
fn real_model_step_only_runs_index_runner_ignored_tests() {
    let workflow = read(TEST_WORKFLOW);
    let steps = named_steps(&workflow);
    let (_, body) = steps
        .iter()
        .find(|(name, _)| name == "Run ignored e2e tests with real e5 model")
        .expect("real model step must exist");
    let command = body
        .lines()
        .find_map(|line| line.trim().strip_prefix("run: "))
        .expect("real model command must exist");
    assert_eq!(
        command, "cargo test -p gwt-core --test index_runner_spawn -- --ignored",
        "Do not run watcher_native or baseline regeneration in the model job"
    );
}

const NIGHTLY_WORKFLOW: &str = ".github/workflows/nightly.yml";
const RUST_JOB: &str = "  test:\n";
const RUN_TESTS_STEP: &str = "Run tests";
const DEFAULT_PARALLEL_JOB: &str = "  test-windows-default-parallel:";
const AGENT_LAUNCH_JOB: &str = "  test-windows-agent-launch-e2e:";
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

/// Issue #4135: consolidation must keep every original integration source.
#[test]
fn consolidated_gwt_suites_register_every_integration_source_once() {
    use std::collections::BTreeSet;

    let manifest: toml::Value =
        toml::from_str(&read("crates/gwt/Cargo.toml")).expect("Cargo manifest");
    assert_eq!(
        manifest["package"]
            .get("autotests")
            .and_then(toml::Value::as_bool),
        Some(false)
    );
    let targets = manifest["test"].as_array().expect("explicit test targets");
    assert_eq!(
        targets.len(),
        10,
        "keep link fan-out bounded to ten harnesses"
    );
    let modules = regex::Regex::new(r#"#\[path\s*=\s*"([^"\n]+)"\]\s*mod\s+\w+\s*;"#)
        .expect("source-module pattern");
    let root = repo_root().join("crates/gwt");
    let originals: BTreeSet<_> = fs::read_dir(root.join("tests"))
        .expect("integration sources")
        .map(|entry| entry.expect("source entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .map(|path| path.canonicalize().expect("original source"))
        .collect();
    let mut registered = BTreeSet::new();
    for target in targets {
        let source = root.join(target["path"].as_str().expect("test path"));
        let contents = fs::read_to_string(&source).expect("test harness source");
        let members = std::iter::once(source.clone()).chain(modules.captures_iter(&contents).map(
            |capture| {
                source
                    .parent()
                    .expect("harness directory")
                    .join(&capture[1])
            },
        ));
        for member in members {
            let member = member.canonicalize().expect("registered source must exist");
            if originals.contains(&member) {
                assert!(registered.insert(member), "source registered twice");
            }
        }
    }
    assert_eq!(
        registered, originals,
        "no original test source may disappear"
    );

    let workflow: serde_yaml::Value =
        serde_yaml::from_str(&read(TEST_WORKFLOW)).expect("test workflow");
    let steps = workflow["jobs"]["changes"]["steps"]
        .as_sequence()
        .expect("classification steps");
    let classify = steps
        .iter()
        .position(|step| step["id"].as_str() == Some("classify"))
        .expect("classification step");
    assert!(
        steps[..classify].iter().any(|step| step["uses"]
            .as_str()
            .is_some_and(|uses| uses.starts_with("actions/checkout@"))
            && step.get("if").is_none()),
        "the consolidated source mapper needs a checkout for pull requests and merge groups"
    );
}

/// Separate integration files now share a process and must share its env lock.
#[test]
fn consolidated_environment_helpers_use_the_shared_core_lock() {
    let accessor = regex::Regex::new(r"(?ms)^fn env_(?:test_)?lock\(\)[^{]*\{(.*?)^\}")
        .expect("environment helper pattern");
    for entry in fs::read_dir(repo_root().join("crates/gwt/tests")).expect("test sources") {
        let path = entry.expect("source entry").path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("test source");
        for helper in accessor.captures_iter(&source) {
            assert!(
                helper[1].contains("gwt_core::test_support::env_lock()"),
                "{} retains a separate process-global environment lock",
                path.display()
            );
        }
    }
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

/// Issue #4839: hosted runners use deterministic contracts, not host measurements.
#[test]
fn hosted_windows_startup_selects_only_the_native_command_contract() {
    let workflow = read(TEST_WORKFLOW);
    let job = job_body(&workflow, "  test-windows-rust:");
    let (_, step) = named_steps(job)
        .into_iter()
        .find(|(name, _)| name == "Check Windows startup tray readiness contract")
        .expect("Windows startup contract step");
    assert!(step.contains("run: node scripts/ci-windows-tests.mjs run gwt test startup_tray_performance startup_metric_and_native_command_contract -- --exact --test-threads=1 --nocapture"));
    assert_eq!(timeout_minutes(&step), Some(1));
    assert!(
        !step.contains("--ignored"),
        "the selected test is not ignored"
    );
    for host_only in [
        "startup_tray_under_large_stopped_session_load",
        "startup_update_resume_under_large_session_load",
    ] {
        assert!(
            !job.contains(host_only),
            "{host_only} is a dev-host measurement"
        );
    }
}

#[test]
fn startup_git_budget_is_required_on_linux_and_windows() {
    let workflow = read(TEST_WORKFLOW);
    assert!(rust_job(&workflow)
        .contains("run: cargo nextest run --workspace --all-features --test-threads=1"));
    let job = job_body(&workflow, "  test-windows-rust:");
    let (_, step) = named_steps(job)
        .into_iter()
        .find(|(name, _)| name == "Check startup update Git-spawn budget")
        .expect("Windows must run the existing 1500-session Git budget test");
    assert!(step.contains("run: node scripts/ci-windows-tests.mjs run gwt bin gwt app_runtime::tests::workspace_resume_tests::startup_restore_update_marker_1500_sessions_bounds_git_spawns -- --exact --test-threads=1"));
    assert!(!step.contains("continue-on-error:"));
    assert!(!step.contains("if:"));
}

#[test]
fn native_tray_measurement_runs_only_on_nightly_by_exact_name() {
    let nightly = read(NIGHTLY_WORKFLOW);
    let job = job_body(&nightly, "  test-windows-startup-tray:");
    assert!(job.contains("cargo test -p gwt --test startup_tray_performance startup_tray_under_large_stopped_session_load -- --exact --ignored --test-threads=1 --nocapture"));
    assert!(!job.contains("startup_update_resume_under_large_session_load"));
    assert!(job.contains("actions/upload-artifact@"));
    assert!(job.contains("if: always()"));
    let reporter = job_body(&nightly, "  report-nightly-failure:");
    assert!(reporter.contains("test-windows-startup-tray"));
}

#[test]
fn linux_infrastructure_regressions_run_beside_the_workspace_suite() {
    let workflow = read(TEST_WORKFLOW);
    let job = job_body(&workflow, "  test-linux-infrastructure:");
    assert!(job.contains("runs-on: ubuntu-latest"));
    assert!(job.contains("shared-key: linux-workspace"));
    assert!(job.contains("cargo-nextest@"));
    assert!(job.contains("scripts/ci-apt.sh gtk-deps"));
    assert!(job.contains("needs: source-sync"));
    assert!(job.contains("if: ${{ !cancelled() }}"));
    let prepare = job
        .find("cargo test -p gwt --all-features --lib --test cli_contracts --no-run")
        .expect("a fresh job must build the guarded gwtd and stress harness");
    assert!(
        prepare
            < job
                .find("scripts/test-gwtd-verification-artifact.py")
                .unwrap()
    );
    for script in [
        "scripts/test-nextest-timeout.mjs",
        "scripts/test-gwtd-verification-artifact.py",
        "scripts/test-managed-assets-lock-stress.py",
    ] {
        assert!(job.contains(script), "parallel job must retain {script}");
        assert!(
            !rust_job(&workflow).contains(script),
            "main job still runs {script}"
        );
    }
    let required = job_body(&workflow, "  test-rust-required:");
    assert!(required.contains("name: Test (Rust)\n"));
    assert!(required.contains(
        "needs: [test, test-linux-infrastructure, test-windows-verify-timings, source-sync]"
    ));
    assert!(required.contains("if: ${{ !cancelled() }}"));
    assert!(required.contains("RUST_RESULT: ${{ needs.test.result }}"));
    assert!(required.contains("INFRA_RESULT: ${{ needs.test-linux-infrastructure.result }}"));
    assert!(required.contains("test \"$RUST_RESULT\" = success"));
    assert!(required.contains("test \"$INFRA_RESULT\" = success"));
}

#[test]
fn paired_windows_timings_preserve_both_artifacts_and_gate_delivery() {
    let workflow = read(TEST_WORKFLOW);
    let job = job_body(&workflow, "  test-windows-verify-timings:");
    assert!(job.contains("runs-on: ${{ needs.changes.outputs.verify_timings == 'false' && 'ubuntu-latest' || 'windows-latest' }}"));
    assert!(job.contains("needs: [changes, source-sync]"));
    assert!(job.contains("if: ${{ !cancelled() }}"));
    // Unknown/failed classification must measure, not silently skip.
    assert!(job.contains("if: ${{ needs.changes.outputs.verify_timings != 'false' }}"));
    assert!(job.contains("cargo-nextest@0.9.146"));
    let (_, measure) = named_steps(job)
        .into_iter()
        .find(|(name, _)| name == "Measure serial and grouped derived gwt-lib schedules")
        .expect("both schedules run in one step on the same host");
    assert!(measure.contains("python scripts/ci_verify_timings.py --output target/verify-timings"));
    assert!(!measure.contains("continue-on-error"));
    assert!(measure.contains("if: ${{ needs.changes.outputs.verify_timings != 'false' }}"));
    let (_, upload) = named_steps(job)
        .into_iter()
        .find(|(name, _)| name == "Upload both measurements even on failure")
        .expect("preserve raw evidence when a measurement fails");
    assert!(
        upload.contains("if: ${{ always() && needs.changes.outputs.verify_timings != 'false' }}")
    );
    assert!(upload.contains("path: target/verify-timings/"));
    assert!(upload.contains("if-no-files-found: error"));
    let required = job_body(&workflow, "  test-rust-required:");
    assert!(required.contains("TIMINGS_RESULT: ${{ needs.test-windows-verify-timings.result }}"));
    assert!(required.contains("if: ${{ !cancelled() }}"));
    assert!(required.contains("case \"$TIMINGS_RESULT\" in success|skipped) ;; *) exit 1 ;; esac"));
}

#[test]
fn windows_filters_use_prebuilt_targets_with_one_build_per_feature_set() {
    let workflow = read(TEST_WORKFLOW);
    let job = job_body(&workflow, "  test-windows-rust:");
    assert_eq!(job.matches("ci-windows-tests.mjs build default").count(), 1);
    assert_eq!(job.matches("ci-windows-tests.mjs build warm").count(), 1);
    assert!(
        !job.contains("cargo test"),
        "filters must execute built binaries"
    );
    let warm = job.find("ci-windows-tests.mjs build warm").unwrap();
    assert!(job.rfind("ci-windows-tests.mjs run ").unwrap() < warm);
    assert!(
        job.find("ci-windows-tests.mjs build default").unwrap()
            < job.find("ci-windows-tests.mjs run ").unwrap()
    );
    assert!(job.contains("ci-windows-tests.mjs run-warm gwt lib gwt cli::hook::event_dispatcher::tests::warm_four_megabyte_history_user_prompt_submit_p95_stays_within_budget"));
    assert!(job.contains("ci-windows-tests.mjs run gwt lib gwt issue_cache::tests::targeted_issue_refresh_writes_one_snapshot_without_marking_full_cache_fresh -- --exact"));
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
/// 137s of tests. The installed-only job still builds once, runs both
/// providers, and attributes a failure to its provider.
#[test]
fn windows_agent_launch_e2e_builds_once_and_runs_installed_providers() {
    let workflow = read(TEST_WORKFLOW);
    let job = job_body(&workflow, AGENT_LAUNCH_JOB);
    assert!(
        !job.contains("matrix:"),
        "the installed provider cases share one cold build, so the job \
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
        "the shared build must precede the installed provider loop"
    );
    assert!(
        job.contains("for provider in codex claude; do"),
        "the shared launch job must run both installed providers"
    );
    assert!(
        job.contains("::error::"),
        "collapsing the matrix must not cost per-provider attribution; a \
         failing provider has to annotate itself"
    );
    assert!(
        job.contains("status=1"),
        "the loop must keep the matrix's fail-fast: false semantics and run \
         every provider before failing"
    );
}
