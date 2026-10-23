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
    assert!(step.contains("run: node scripts/ci-windows-tests.mjs run-runtime gwt test startup_tray_performance startup_metric_and_native_command_contract -- --exact --test-threads=1 --nocapture"));
    assert_eq!(timeout_minutes(&step), Some(1));
    assert!(!step.contains("if:"));
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
    assert!(step.contains("run: node scripts/ci-windows-tests.mjs run-runtime gwt bin gwt app_runtime::tests::workspace_resume_tests::startup_restore_update_marker_1500_sessions_bounds_git_spawns -- --exact --test-threads=1"));
    assert!(!step.contains("continue-on-error:"));
    assert!(!step.contains("if:"));
}

/// Issue #5564: slices must stay independent and their union must remain required.
#[test]
fn workspace_shards_preserve_the_complete_serial_test_selection() {
    let workflow: serde_yaml::Value =
        serde_yaml::from_str(&read(TEST_WORKFLOW)).expect("test workflow");
    let job = &workflow["jobs"]["test"];
    assert_eq!(job["strategy"]["fail-fast"].as_bool(), Some(false));
    let shards: Vec<_> = job["strategy"]["matrix"]["shard"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|value| value.as_u64().unwrap())
        .collect();
    assert_eq!(shards, vec![1, 2, 3, 4, 5, 6]);
    assert!(job["concurrency"]["group"]
        .as_str()
        .unwrap()
        .contains("matrix.shard"));
    let steps = job["steps"].as_sequence().unwrap();
    let step = |name| {
        steps
            .iter()
            .find(|step| step["name"].as_str() == Some(name))
            .unwrap()
    };
    assert!(step("Build and inventory workspace tests")["run"]
        .as_str()
        .unwrap()
        .contains("cargo nextest list --workspace --all-features --message-format json"));
    assert_eq!(step(RUN_TESTS_STEP)["run"].as_str(), Some(
        "cargo nextest run --workspace --all-features --test-threads=1 --profile ci --partition slice:${{ matrix.shard }}/6"));
    for name in [
        "Run doctests",
        "Run real-binary front-door tests (Xvfb)",
        "Voice accuracy regression (optional local dataset)",
    ] {
        assert_eq!(step(name)["if"].as_str(), Some("matrix.shard == 1"));
    }
    let upload = step("Upload workspace test inventory and results");
    assert_eq!(upload["if"].as_str(), Some("${{ always() }}"));
    assert_eq!(upload["with"]["if-no-files-found"].as_str(), Some("error"));
    let raw = read(TEST_WORKFLOW);
    let required = job_body(&raw, "  test-rust-required:");
    assert!(required.contains("pattern: workspace-tests-*"));
    assert!(required
        .contains("python3 scripts/ci_workspace_tests.py target/workspace-shards --shards 6"));
    assert!(!required.contains("continue-on-error"));
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

/// Issue #5437: Windows must exercise the real model across every search scope.
#[test]
fn nightly_windows_index_e2e_runs_real_scopes_and_background_contention() {
    let nightly = read(NIGHTLY_WORKFLOW);
    let job = job_body(&nightly, "  test-windows-index-e2e:");
    assert!(job.contains("runs-on: windows-latest"));
    assert!(job.contains("ref: develop"));
    assert!(
        !job.contains("if:"),
        "index verification must not be skipped"
    );
    assert!(
        !job.contains("continue-on-error:"),
        "real-model failures must fail the nightly run"
    );
    let build = job
        .find("cargo test -p gwt --test runtime_tests --no-run")
        .expect("compile the runtime harness before either measurement");
    for test in [
        "index_resource_harness_test::real_e5_all_scopes_build_and_search",
        "index_resource_harness_test::real_warm_search_finishes_during_background_build_and_build_resumes",
    ] {
        let command = format!(
            "cargo test -p gwt --test runtime_tests {test} -- --exact --ignored --test-threads=1 --nocapture"
        );
        let run = job.find(&command).unwrap_or_else(|| panic!("missing {test}"));
        assert!(build < run, "compile before running {test}");
    }
    let steps = named_steps(job);
    let (_, benchmark) = steps
        .iter()
        .find(|(name, _)| name == "Measure alternating scope searches with real E5")
        .expect("Windows must run the existing alternating-scope 2s benchmark");
    assert!(benchmark.contains("GWT_INDEX_REAL_SEARCH_BENCHMARK: \"1\""));
    assert!(benchmark.contains("HF_HUB_CACHE: ${{ env.HF_HOME }}/hub"));
    assert!(benchmark.contains("working-directory: crates/gwt-core/runtime"));
    assert!(benchmark.contains("run: python -m unittest discover -s tests -p test_search_multi_contract.py -k test_cli_searches_reuse_worker_and_reopen_published_store -v"));
    let reporter = job_body(&nightly, "  report-nightly-failure:");
    let needs = reporter
        .lines()
        .find(|line| line.trim_start().starts_with("needs:"))
        .expect("nightly failure dependencies");
    assert!(needs.contains("test-windows-index-e2e"));
    assert!(
        reporter.contains("real-model index measurement"),
        "failure reports must describe index verification"
    );
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
    assert!(required.contains("needs: [test, test-linux-infrastructure, source-sync]"));
    assert!(required.contains("if: ${{ !cancelled() }}"));
    assert!(required.contains("RUST_RESULT: ${{ needs.test.result }}"));
    assert!(required.contains("INFRA_RESULT: ${{ needs.test-linux-infrastructure.result }}"));
    assert!(required.contains("test \"$RUST_RESULT\" = success"));
    assert!(required.contains("test \"$INFRA_RESULT\" = success"));
}

#[test]
fn paired_windows_timings_preserve_evidence_outside_the_pr_path() {
    let pr_workflow = read(TEST_WORKFLOW);
    assert!(!pr_workflow.contains("test-windows-verify-timings"));
    assert!(!pr_workflow.contains("verify_timings"));
    let workflow = read(NIGHTLY_WORKFLOW);
    assert!(workflow.contains("schedule:"));
    assert!(workflow.contains("workflow_dispatch:"));
    assert!(!workflow.contains("pull_request:"));
    let job = job_body(&workflow, "  test-windows-verify-timings:");
    assert!(job.contains("runs-on: windows-latest"));
    assert!(job.contains("ref: develop"));
    assert!(job.contains("fetch-depth: 0"));
    assert!(job.contains("cargo-nextest@0.9.146"));
    let (_, measure) = named_steps(job)
        .into_iter()
        .find(|(name, _)| name == "Measure serial and grouped derived gwt-lib schedules")
        .expect("both schedules run in one step on the same host");
    assert!(measure.contains("python scripts/ci_verify_timings.py --output target/verify-timings"));
    assert!(!measure.contains("continue-on-error"));
    assert!(!measure.contains("if:"));
    let (_, upload) = named_steps(job)
        .into_iter()
        .find(|(name, _)| name == "Upload both measurements even on failure")
        .expect("preserve raw evidence when a measurement fails");
    assert!(upload.contains("if: always()"));
    assert!(upload.contains("path: target/verify-timings/"));
    assert!(upload.contains("if-no-files-found: error"));
    let report = job_body(&workflow, "  report-nightly-failure:");
    assert!(report
        .lines()
        .any(|line| line.contains("needs:") && line.contains("test-windows-verify-timings")));
    assert!(report.contains("if: failure()"));
}

#[test]
fn windows_regression_slices_build_each_harness_once_and_gate_all_filters() {
    use serde_yaml::Value;

    let workflow = read(TEST_WORKFLOW);
    let warm = job_body(&workflow, "  test-windows-warm:");
    assert!(!workflow.contains("test-windows-regression-build:"));
    assert_eq!(
        workflow.matches("ci-windows-tests.mjs build warm").count(),
        1
    );
    assert_eq!(warm.matches("ci-windows-tests.mjs build warm").count(), 1);
    assert!(warm.contains("needs: source-sync"));
    assert!(
        warm.find("ci-windows-tests.mjs build warm").unwrap()
            < warm.find("ci-windows-tests.mjs run-warm").unwrap()
    );
    assert!(warm.contains("ci-windows-tests.mjs run-warm gwt lib gwt cli::hook::event_dispatcher::tests::warm_four_megabyte_history_user_prompt_submit_p95_stays_within_budget -- --exact --test-threads=1"));

    let doc: Value = serde_yaml::from_str(&workflow).expect("Test workflow");
    let invocation =
        regex::Regex::new(r"node scripts/ci-windows-tests\.mjs (run(?:-lib|-runtime)?) ([^\r\n]+)")
            .expect("default regression invocation");
    let mut actual_filters = Vec::new();
    for (name, group, command, lib) in [
        ("test-windows-rust", "default-runtime", "run-runtime", false),
        ("test-windows-rust-lib", "default-lib", "run-lib", true),
    ] {
        let job = &doc["jobs"][name];
        assert_eq!(
            job["needs"].as_str(),
            Some("source-sync"),
            "{name} must start independently"
        );
        assert!(job["strategy"].is_null(), "{name} builds each harness once");
        assert!(job["env"]["CARGO_PROFILE_DEV_DEBUG"].is_null());
        let steps = job["steps"].as_sequence().expect("regression steps");
        assert_eq!(
            steps[0]["with"]["ref"].as_str(),
            Some("${{ github.event.pull_request.head.sha || github.sha }}"),
            "T453 native evidence must use the exact PR head"
        );
        let build_command = format!("node scripts/ci-windows-tests.mjs build {group}");
        assert_eq!(workflow.matches(&build_command).count(), 1);
        let build = steps
            .iter()
            .position(|step| step["run"].as_str() == Some(build_command.as_str()))
            .expect("each regression group needs its own build");
        assert_eq!(
            steps[build]["env"]["CARGO_PROFILE_DEV_DEBUG"].as_str(),
            Some("0")
        );
        assert!(steps[build]["if"].is_null());
        // Issue #1921 T453: exact acceptance precedes each slice's broader
        // regressions and cannot pass with a renamed or ignored test.
        let exact = &steps[build + 1];
        assert!(exact["name"]
            .as_str()
            .unwrap()
            .starts_with("Run T453 exact Windows"));
        assert_eq!(exact["shell"].as_str(), Some("bash"));
        assert!(exact["if"].is_null());
        assert!(exact["continue-on-error"].is_null());
        let exact_run = exact["run"].as_str().unwrap();
        assert!(exact_run.contains("\"$@\" | tee \"$RUNNER_TEMP/t453-exact.log\""));
        assert!(exact_run.contains("grep -Fq 'test result: ok. 1 passed; 0 failed; 0 ignored;'"));
        assert_eq!(
            exact_run
                .matches("exact node scripts/ci-windows-tests.mjs")
                .count(),
            7
        );
        assert_eq!(
            exact_run
                .matches(" -- --exact --test-threads=1 --nocapture")
                .count(),
            7
        );
        if lib {
            let fleet = steps
                .iter()
                .find(|step| {
                    step["name"].as_str()
                        == Some("Run native fleet orphan process handle contracts")
                })
                .expect("Windows must execute both fleet process handle contracts");
            assert!(
                fleet["run"]
                    .as_str()
                    .unwrap()
                    .contains("grep -Fq 'test result: ok. 2 passed; 0 failed; 0 ignored;'"),
                "the native handle filter must not pass with zero matches"
            );
        }
        for (index, step) in steps.iter().enumerate() {
            if step["uses"].as_str() == Some("Swatinem/rust-cache@v2") {
                assert!(step["env"]["CARGO_PROFILE_DEV_DEBUG"].is_null());
            }
            let Some(run) = step["run"].as_str() else {
                continue;
            };
            assert!(
                !run.contains("cargo test"),
                "filters execute prebuilt binaries"
            );
            for capture in invocation.captures_iter(run) {
                assert!(build < index, "{name} must build before running filters");
                assert!(
                    step["if"].is_null(),
                    "all regression filters run unconditionally"
                );
                assert_eq!(&capture[1], command);
                let filter = &capture[2];
                assert_eq!(filter.split_whitespace().nth(1) == Some("lib"), lib);
                actual_filters.push(filter.to_owned());
            }
        }
    }

    let required = &doc["jobs"]["test-windows-required"];
    let needs = required["needs"]
        .as_sequence()
        .expect("protected Windows dependencies");
    let gate = required["steps"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|step| step["env"]["WINDOWS_RESULT"].as_str().is_some())
        .expect("protected Windows result gate");
    let gate_run = gate["run"].as_str().unwrap();
    for name in ["test-windows-rust", "test-windows-rust-lib"] {
        assert!(needs.iter().any(|need| need.as_str() == Some(name)));
        let result = format!("${{{{ needs.{name}.result }}}}");
        let key = gate["env"]
            .as_mapping()
            .unwrap()
            .iter()
            .find_map(|(key, value)| {
                (value.as_str() == Some(result.as_str())).then(|| key.as_str().unwrap())
            })
            .expect("both regression results must reach the protected gate");
        assert!(gate_run.contains(&format!("test \"${key}\" = success")));
    }

    let runtime = &doc["jobs"]["test-windows-rust"];
    let driver = runtime["steps"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|step| {
            step["run"]
                .as_str()
                .is_some_and(|run| run.contains("verification_driver_cli_test::"))
        })
        .expect("five workers must retain all ten real-artifact repetitions");
    assert!(driver["if"].is_null());
    let driver_run = driver["run"].as_str().unwrap();
    assert!(driver_run.contains("1..5 | ForEach-Object"));
    assert!(driver_run.contains("Start-Job"));
    assert!(driver_run.contains("$attempt -le 2"));
    assert!(driver_run.contains("verification_driver_cli_test:: -- --test-threads=1"));
    assert_eq!(
        driver_run.matches("verification_driver_cli_test::").count(),
        1
    );
    assert!(!driver_run.contains("cli::verification_record::driver::"));
    assert!(
        driver_run.contains("throw"),
        "native command failures must fail their worker"
    );
    assert!(driver_run.contains("Wait-Job") && driver_run.contains("Receive-Job"));
    assert!(
        driver_run.contains(".State -ne 'Completed'")
            || driver_run.contains("Where-Object State -ne Completed")
    );
    assert!(
        driver_run.contains("exit 1"),
        "any failed worker must fail the required job"
    );

    // Issue #5565 split the original 49 default filters across both hosts.
    // Issue #5560 adds the FIFO regression; warm keeps its separate filter above.
    let mut expected_filters = vec![
        "gwt lib gwt cli::fleet_disposition::tests::orphan_process_handle_ -- --test-threads=1 --nocapture",
        "gwt lib gwt issue_cache::tests::targeted_issue_refresh_writes_one_snapshot_without_marking_full_cache_fresh -- --exact",
        "gwt bin gwt app_runtime::tests::continuation_completion_tests::startup_self_heal_converges_legacy_config_without_a_runtime_guard -- --exact --test-threads=1",
        "gwt test hook_contracts hook_health_test::codex_hook_audit_and_repair_cover_worktree_local_and_workspace_home_copies -- --exact --test-threads=1",
        "gwt bin gwt dispatch_watchdog::tests:: -- --test-threads=1",
        "gwt-core test process_priority_roundtrip -- --nocapture",
        "gwt-core test windows_process_resolver",
        "gwt-core test process_adapter_parity",
        "gwt-core test windows_claude_user_agent",
        "gwt-agent lib gwt_agent real_bun_global_placeholder_fixture",
        "gwt-agent lib gwt_agent direct_runner_resolution_failure_still_emits_an_end_summary",
        "gwt bin gwt real_bun_global_placeholder_fixture",
        "gwt bin gwt command_prompt_agent_wrapper",
        "gwt-terminal lib gwt_terminal pty::windows_spawn::tests -- --test-threads=1",
        "gwt-agent lib gwt_agent claude_child_session_marker -- --test-threads=1",
        "gwt test ci_contracts windows_binary_subsystem_test:: -- --test-threads=1",
        "gwt test runtime_tests pty_start_gate_console_test:: -- --test-threads=1",
        "gwt-terminal test pty_start_gate_test -- --test-threads=1",
        "gwt bin gwt scheduled_tick_is_single_flight_per_canonical_project_scope -- --test-threads=1",
        "gwt bin gwt scheduled_tick_drives_disabled_claim_cleanup_without_scanning -- --test-threads=1",
        "gwt bin gwt scheduled_tick_advances_autonomous_launch_without_an_external_daemon -- --test-threads=1",
        "gwt bin gwt app_runtime_local_driver_ -- --test-threads=1",
        "gwt bin gwt app_runtime::tests::issue_monitor_claim_tests::app_runtime_full_issue_monitor_scan_migrates_legacy_git_failure_and_persists_marker -- --exact --test-threads=1",
        "gwt lib gwt local_fallback_lease -- --test-threads=1",
        "gwt lib gwt cli::issue::tests::windows_launch_now_ -- --test-threads=1",
        "gwt lib gwt cli::issue::tests::immediate_scan_delivery_never_claims_an_unacknowledged_schedule -- --exact --test-threads=1",
        "gwt lib gwt cli::pane::tests::pane_websocket_request_carries_the_agent_capability_in_authorization -- --exact --test-threads=1",
        "gwt lib gwt cli::pane::tests::issue_monitor_scan_client_ -- --test-threads=1",
        "gwt bin gwt embedded_server::tests::authenticated_monitor_scan_routes_scope_guard_and_result_only_to_origin_socket -- --exact --test-threads=1",
        "gwt bin gwt authenticated_pm_scan_now_ -- --test-threads=1",
        "gwt bin gwt authenticated_scan_now_ -- --test-threads=1",
        "gwt test coordination_tests issue_monitor_protocol_test::frontend_issue_monitor_events_use_snake_case_wire_shape -- --exact --test-threads=1",
        "gwt test coordination_tests issue_monitor_protocol_test::agent_issue_monitor_scan_ -- --test-threads=1",
        "gwt test runtime_tests agent_process_resolution_contract_test::",
        "gwt lib gwt cli::daemon::transport -- --test-threads=1",
        "gwt lib gwt cli::daemon::client -- --test-threads=1",
        "gwt lib gwt cli::daemon::tests -- --test-threads=1",
        "gwt lib gwt daemon_subscriber -- --test-threads=1",
        "gwt lib gwt daemon_supervisor -- --test-threads=1",
        "gwt lib gwt process::tests -- --test-threads=1",
        "gwt-core lib gwt_core daemon_pipe_name -- --test-threads=1",
        "gwt lib gwt cli::verification_record::tests::verification_records_exit_while_grandchild_holds_output -- --exact --test-threads=1",
        "gwt lib gwt cli::verification_lease:: -- --test-threads=1",
        "gwt lib gwt cli::verification_record::driver:: -- --test-threads=1",
        "gwt test cli_contracts verification_driver_cli_test:: -- --test-threads=1",
        "gwt test cli_contracts verification_lease_cli_test::deferred_fifo_survives_status_read_contention_and_new_processes -- --exact --nocapture",
        "gwt lib gwt cli::issue::tests::monitor_wait_ -- --test-threads=1",
        "gwt lib gwt issue_monitor::tests::wait_declaration_ -- --test-threads=1",
        "gwt lib gwt issue_monitor::tests::agent_status_projects_the_wait_declaration -- --exact --test-threads=1",
        "gwt test startup_tray_performance startup_metric_and_native_command_contract -- --exact --test-threads=1 --nocapture",
        "gwt bin gwt app_runtime::tests::workspace_resume_tests::startup_restore_update_marker_1500_sessions_bounds_git_spawns -- --exact --test-threads=1",
        "gwt lib gwt cli::env::tests::dispatch_json_envelope_hook_doctor_can_repair_missing_managed_configs -- --exact --test-threads=1 --nocapture",
        "gwt lib gwt cli::env::tests::dispatch_json_envelope_hook_health_returns_managed_health_json -- --exact --test-threads=1 --nocapture",
        "gwt lib gwt cli::env::tests::hook_doctor_repair_does_not_persist_path_local_build_binary -- --exact --test-threads=1 --nocapture",
        "gwt lib gwt cli::env::tests::new_for_hooks_keeps_detached_cache_root -- --exact --test-threads=1 --nocapture",
        "gwt lib gwt cli::env::tests::owner_runtime_override_requires_complete_explicit_loopback_environment -- --exact --test-threads=1 --nocapture",
        "gwt lib gwt cli::execution_state::tests::command::repair_recovers_mirror_pointer_partial_authority_advertised_by_status -- --exact --test-threads=1 --nocapture",
        "gwt lib gwt cli::execution_state::tests::command::repair_recovers_malformed_mirror_only_ecr -- --exact --test-threads=1 --nocapture",
        "gwt bin gwt app_runtime::tests::continuation_authority_tests::continue_work_nonlocal_liveness_distinguishes_live_dead_and_stopped_owners -- --exact --test-threads=1 --nocapture",
        "gwt bin gwt tests::powershell_agent_wrapper_quotes_spaced_path_and_single_quotes -- --exact --test-threads=1 --nocapture",
        "gwt bin gwt app_runtime::tests::continuation_completion_tests::startup_self_heal_converges_legacy_config_to_explicit_hook_binary_pin -- --exact --test-threads=1 --nocapture",
        "gwt test hook_contracts hook_health_test::managed_hook_health_understands_runtime_indirect_fallbacks -- --exact --test-threads=1 --nocapture",
        "gwt test hook_contracts hook_health_test::managed_hook_health_understands_all_provider_runtime_indirect_fallbacks -- --exact --test-threads=1 --nocapture",
        "gwt test cli_contracts discussion_cli_test::discussion_update_creates_single_canonical_discussions_file -- --exact --test-threads=1 --nocapture",
        "gwt test cli_contracts memory_cli_test::memory_add_writes_to_machine_local_work_notes -- --exact --test-threads=1 --nocapture",
    ];
    assert_eq!(expected_filters.len(), 65);
    expected_filters.sort_unstable();
    actual_filters.sort_unstable();
    assert_eq!(actual_filters, expected_filters);
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
    use serde_yaml::Value;

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
    let doc: Value = serde_yaml::from_str(&workflow).expect("Test workflow");
    let agent_job = &doc["jobs"]["test-windows-agent-launch-e2e"];
    assert!(agent_job["env"]["CARGO_PROFILE_DEV_DEBUG"].is_null());
    let steps = agent_job["steps"].as_sequence().unwrap();
    let cargo_steps: Vec<_> = steps
        .iter()
        .filter(|step| {
            step["run"]
                .as_str()
                .is_some_and(|run| run.contains("cargo test"))
        })
        .collect();
    assert_eq!(cargo_steps.len(), 3);
    assert!(cargo_steps
        .iter()
        .all(|step| step["env"]["CARGO_PROFILE_DEV_DEBUG"].as_str() == Some("0")));
    assert!(steps.iter().all(|step| {
        step["uses"].as_str() != Some("Swatinem/rust-cache@v2")
            || step["env"]["CARGO_PROFILE_DEV_DEBUG"].is_null()
    }));
    let providers = cargo_steps
        .iter()
        .find_map(|step| {
            let run = step["run"].as_str()?;
            run.contains("--ignored").then_some(run)
        })
        .unwrap();
    assert!(providers.contains("providers=(codex claude)"));
    assert!(providers.contains("pids=()"));
    assert!(providers.contains("2>&1 &"));
    assert!(providers.contains("pids+=(\"$!\")"));
    assert!(providers.contains("for index in \"${!pids[@]}\"; do"));
    assert!(providers.contains("provider=\"${providers[$index]}\""));
    assert!(providers.contains("if wait \"${pids[$index]}\"; then"));
    assert!(providers.contains("cat \"$RUNNER_TEMP/windows-agent-${provider}.log\""));
    assert!(providers.contains("exit \"$status\""));
}
