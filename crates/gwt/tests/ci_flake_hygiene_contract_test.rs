//! Contract tests for the CI half of the test-hygiene gates (SPEC #4551).
//!
//! `crates/gwt-core/tests/test_hygiene_test.rs` rejects mechanisms (A) and (B)
//! statically, at the moment the offending line is written. Mechanism (C) —
//! a child process or resource the suite never reaps — has no textual shape to
//! scan for: it only exists while the suite runs. So does a flake, which is by
//! definition a test whose outcome is not a function of its source.
//!
//! Both therefore live in `test.yml` as runtime steps, and these tests pin the
//! wiring: a gate nobody invokes is indistinguishable from no gate at all.
//!
//! - T-050 / AC-5: `Test (Rust)` fails when the suite leaves a process from the
//!   build tree alive behind it.
//! - T-055 / AC-6: a separate job re-runs the changed crates' tests at default
//!   parallelism and fails when the outcome is not stable across runs.
//!
//! The flake job must stay off the required-status-check list. Branch
//! protection reports a skipped job as Success, so a required check gated on
//! `changes` would let a PR merge without ever having run it — the same
//! property `ci_concurrency_contract_test.rs` pins for the docs-only filter.

use serde_yaml::Value;
use std::fs;
use std::path::{Path, PathBuf};

const TEST_WORKFLOW: &str = ".github/workflows/test.yml";
const ORPHAN_SCRIPT: &str = "scripts/ci-check-orphan-processes.sh";
const FLAKE_SCRIPT: &str = "scripts/ci-flake-detect.sh";
const CHANGES_JOB: &str = "changes";
const RUST_TEST_JOB: &str = "test";
const FLAKE_JOB: &str = "flake-detection";
const CRATES_OUTPUT: &str = "crates";

/// SPEC #4551 plan: "N = 20, 対象は変更されたクレートの test target のみ, 毎 PR".
const REQUIRED_FLAKE_RUNS: u32 = 20;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("gwt crate must be nested under crates/")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn test_workflow() -> Value {
    serde_yaml::from_str(&read(TEST_WORKFLOW)).expect("test.yml must be valid YAML")
}

fn job(doc: &Value, id: &str) -> Value {
    doc.get("jobs")
        .and_then(|jobs| jobs.get(id))
        .cloned()
        .unwrap_or_else(|| panic!("{TEST_WORKFLOW} must keep the `{id}` job"))
}

/// The `run:` bodies of a job's steps, in declaration order.
fn run_steps(job: &Value) -> Vec<(String, String)> {
    job.get("steps")
        .and_then(Value::as_sequence)
        .unwrap_or_else(|| panic!("a job must declare steps"))
        .iter()
        .filter_map(|step| {
            let run = step.get("run").and_then(Value::as_str)?;
            let name = step
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("<unnamed>");
            Some((name.to_string(), run.to_string()))
        })
        .collect()
}

fn index_of_step_running(steps: &[(String, String)], needle: &str) -> Option<usize> {
    steps.iter().position(|(_, run)| run.contains(needle))
}

/// T-050 / AC-5: the check exists, and it runs where it can still see what the
/// suite left behind — in the same job, after the suite.
#[test]
fn the_rust_test_job_checks_for_processes_the_suite_failed_to_reap() {
    let doc = test_workflow();
    let steps = run_steps(&job(&doc, RUST_TEST_JOB));

    let suite = index_of_step_running(&steps, "cargo test --workspace")
        .expect("`Test (Rust)` must keep the workspace `cargo test` step");
    let orphan_check = index_of_step_running(&steps, ORPHAN_SCRIPT).unwrap_or_else(|| {
        panic!("`Test (Rust)` must run {ORPHAN_SCRIPT} (SPEC #4551 T-050 / AC-5)")
    });

    assert!(
        orphan_check > suite,
        "{ORPHAN_SCRIPT} must run after the suite it inspects; \
         suite at step {suite}, check at step {orphan_check}"
    );
}

/// The orphan check must report what it found. A step that fails with no
/// process list turns a diagnosable leak into an unexplained red run, which is
/// the failure mode SPEC #4551 exists to remove.
#[test]
fn the_orphan_process_check_reports_the_surviving_command_lines() {
    let script = read(ORPHAN_SCRIPT);
    assert!(
        script.contains("cmdline") || script.contains("args"),
        "{ORPHAN_SCRIPT} must print the surviving command lines"
    );
    assert!(
        script.contains("exit 1"),
        "{ORPHAN_SCRIPT} must fail the step when a process survives"
    );
}

/// T-055 / AC-6: the classification job already walks the PR's files, so the
/// flake job reads the changed crates from it rather than paying for a second
/// pass — and rather than re-running the whole workspace twenty times.
#[test]
fn the_change_classifier_publishes_the_changed_crates() {
    let doc = test_workflow();
    let outputs = job(&doc, CHANGES_JOB)
        .get("outputs")
        .and_then(Value::as_mapping)
        .cloned()
        .expect("the `changes` job must declare outputs");

    assert!(
        outputs.contains_key(Value::String(CRATES_OUTPUT.to_string())),
        "the `changes` job must publish a `{CRATES_OUTPUT}` output (SPEC #4551 T-055)"
    );
}

/// T-055 / AC-6: the job exists, is fed by the classifier, and runs the
/// repeat-until-it-wobbles script.
#[test]
fn a_flake_detection_job_reruns_the_changed_crates() {
    let doc = test_workflow();
    let flake = job(&doc, FLAKE_JOB);

    let needs = match flake.get("needs") {
        Some(Value::String(one)) => vec![one.clone()],
        Some(Value::Sequence(many)) => many
            .iter()
            .filter_map(|n| n.as_str().map(str::to_string))
            .collect(),
        other => panic!("`{FLAKE_JOB}` must declare `needs`, got {other:?}"),
    };
    assert!(
        needs.iter().any(|n| n == CHANGES_JOB),
        "`{FLAKE_JOB}` must consume the `{CHANGES_JOB}` classification, got {needs:?}"
    );

    let steps = run_steps(&flake);
    assert!(
        index_of_step_running(&steps, FLAKE_SCRIPT).is_some(),
        "`{FLAKE_JOB}` must run {FLAKE_SCRIPT}"
    );

    let condition = flake
        .get("if")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("`{FLAKE_JOB}` must be conditional"));
    assert!(
        condition.contains("!cancelled()"),
        "`{FLAKE_JOB}`: a failed classification must fall open, got {condition:?}"
    );
    assert!(
        condition.contains(&format!("needs.{CHANGES_JOB}.outputs.{CRATES_OUTPUT}")),
        "`{FLAKE_JOB}` must skip when no crate changed, got {condition:?}"
    );
}

/// The flake job is gated on `changes`, so it must never carry the name of a
/// required status check: branch protection reads a skipped job as Success.
#[test]
fn the_flake_detection_job_is_not_a_required_status_check() {
    let doc = test_workflow();
    let name = job(&doc, FLAKE_JOB)
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(FLAKE_JOB)
        .to_string();

    // Mirror of develop's `required_status_checks.contexts`; kept in step with
    // `ci_concurrency_contract_test.rs::REQUIRED_CHECKS`.
    const REQUIRED_CHECKS: &[&str] = &[
        "Commit Message Lint",
        "Clippy & Rustfmt",
        "Test (Rust)",
        "Build",
        "Test (Python runner)",
        "Test (Rust, Windows)",
        "Cargo Deny (advisories + sources)",
        "Check (Windows)",
        "Check (macOS)",
        "Clippy (macOS)",
    ];
    assert!(
        !REQUIRED_CHECKS.contains(&name.as_str()),
        "`{FLAKE_JOB}` is gated on `{CHANGES_JOB}`, so `{name}` must not be a required check"
    );
}

/// AC-6 fixes N. A default the workflow does not override is the value that
/// actually runs, so the number lives in one place and is pinned here.
#[test]
fn the_flake_detector_repeats_the_agreed_number_of_times() {
    let script = read(FLAKE_SCRIPT);
    assert!(
        script.contains(&format!("GWT_FLAKE_RUNS:-{REQUIRED_FLAKE_RUNS}")),
        "{FLAKE_SCRIPT} must default to {REQUIRED_FLAKE_RUNS} runs (SPEC #4551 AC-6)"
    );

    let workflow = read(TEST_WORKFLOW);
    assert!(
        !workflow.contains("GWT_FLAKE_RUNS"),
        "the workflow must not override the run count; keep N in {FLAKE_SCRIPT}"
    );
}

/// A gate that cannot be reproduced outside CI gets disabled instead of fixed.
/// Both scripts are plain executables so an agent can run them locally against
/// a deliberately flaky test.
#[test]
fn both_gate_scripts_are_executable() {
    for relative in [ORPHAN_SCRIPT, FLAKE_SCRIPT] {
        let path = repo_root().join(relative);
        assert!(path.is_file(), "{relative} must exist");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path)
                .unwrap_or_else(|error| panic!("stat {relative}: {error}"))
                .permissions()
                .mode();
            assert!(
                mode & 0o111 != 0,
                "{relative} must be executable, got mode {mode:o}"
            );
        }
    }
}
