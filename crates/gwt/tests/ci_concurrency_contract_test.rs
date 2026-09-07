//! Contract tests for CI queue capacity control (Issue #4119).
//!
//! On 2026-09-07 12:00Z, 19 open PRs × 17 checks put 36 runs into the GitHub
//! Actions queue: 35 queued, 0 in progress, and the top-priority PR (#4116)
//! did not start a single check for over 15 minutes. Every develop merge turns
//! every other PR `BEHIND` (branch protection is `strict`), every
//! update-branch re-runs all 17 checks, and nothing cancelled the superseded
//! runs, so one branch held 3–5 queued runs at once. Measured on the jobs API
//! for the 212 jobs created between 11:43Z and 12:40Z, queue latency was
//! p90 33.6 min and max 40.6 min.
//!
//! Two GitHub Actions features contain this: a per-PR `concurrency` group with
//! `cancel-in-progress` so a `synchronize` supersedes the previous run, and a
//! path filter that keeps docs-only changes off the heavy Windows jobs and the
//! WebView E2E job. These tests pin both, plus the property that the filter
//! never touches a required status check.

use serde_yaml::Value;
use std::fs;
use std::path::PathBuf;

const WORKFLOWS: &str = ".github/workflows";
const TEST_WORKFLOW: &str = "test.yml";
const RELEASE_WORKFLOW: &str = "release.yml";
const CHANGES_JOB: &str = "changes";
const DOCS_ONLY_OUTPUT: &str = "needs.changes.outputs.docs_only";

/// Mirror of the develop branch protection `required_status_checks.contexts`
/// (`gh api repos/akiojin/gwt/branches/develop/protection`, 2026-09-07). A job
/// carrying one of these names must run unconditionally: GitHub reports a
/// skipped job as passing, so a required check gated by the path filter would
/// let a PR merge without ever running it.
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
];

/// Heavy, non-required jobs in `test.yml` that a documentation edit cannot
/// affect: four Windows agent launch matrix runs, the three-pass Windows
/// default-parallel job, and the WebView E2E job.
const DOCS_SKIPPABLE_JOBS: &[&str] = &[
    "test-windows-agent-launch-e2e",
    "test-windows-default-parallel",
    "test-frontend",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("gwt crate must be nested under crates/")
        .to_path_buf()
}

fn workflows() -> Vec<(String, Value)> {
    let dir = repo_root().join(WORKFLOWS);
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .map(|entry| entry.expect("workflow dir entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".yml"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let path = dir.join(&name);
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            let doc: Value = serde_yaml::from_str(&text)
                .unwrap_or_else(|error| panic!("{name} must be valid YAML: {error}"));
            (name, doc)
        })
        .collect()
}

fn workflow(name: &str) -> Value {
    workflows()
        .into_iter()
        .find(|(file, _)| file == name)
        .map(|(_, doc)| doc)
        .unwrap_or_else(|| panic!("{WORKFLOWS}/{name} must exist"))
}

/// The `on:` block. YAML 1.1 parsers resolve the bare key `on` to `true`, so
/// both spellings are accepted.
fn triggers(doc: &Value) -> &Value {
    doc.get("on")
        .or_else(|| doc.get(Value::Bool(true)))
        .expect("every workflow declares its triggers")
}

fn triggered_by_pull_request(doc: &Value) -> bool {
    let is_pr = |event: &str| event == "pull_request" || event == "pull_request_target";
    match triggers(doc) {
        Value::String(event) => is_pr(event),
        Value::Sequence(events) => events.iter().any(|e| e.as_str().is_some_and(is_pr)),
        Value::Mapping(events) => events.keys().any(|e| e.as_str().is_some_and(is_pr)),
        other => panic!("unexpected `on` shape: {other:?}"),
    }
}

fn concurrency(name: &str, doc: &Value) -> Value {
    doc.get("concurrency").cloned().unwrap_or_else(|| {
        panic!("{name} must declare a top-level `concurrency` group (Issue #4119 AC-1)")
    })
}

fn cancel_in_progress(name: &str, block: &Value) -> bool {
    block
        .get("cancel-in-progress")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("{name}: `concurrency.cancel-in-progress` must be a boolean"))
}

fn jobs(doc: &Value) -> &serde_yaml::Mapping {
    doc.get("jobs")
        .and_then(Value::as_mapping)
        .expect("every workflow declares jobs")
}

fn job<'a>(doc: &'a Value, id: &str) -> &'a Value {
    jobs(doc)
        .get(id)
        .unwrap_or_else(|| panic!("{TEST_WORKFLOW} must keep the `{id}` job"))
}

fn needs(job: &Value) -> Vec<String> {
    match job.get("needs") {
        None => Vec::new(),
        Some(Value::String(one)) => vec![one.clone()],
        Some(Value::Sequence(many)) => many
            .iter()
            .map(|n| n.as_str().expect("needs entries are job ids").to_string())
            .collect(),
        Some(other) => panic!("unexpected `needs` shape: {other:?}"),
    }
}

fn condition(job: &Value) -> String {
    match job.get("if") {
        None => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(other) => panic!("unexpected `if` shape: {other:?}"),
    }
}

/// AC-1: every workflow declares a concurrency group, so no trigger can stack
/// an unbounded number of runs for the same ref.
#[test]
fn every_workflow_declares_a_concurrency_group() {
    for (name, doc) in workflows() {
        let block = concurrency(&name, &doc);
        let group = block
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{name}: `concurrency.group` must be a string"));
        assert!(
            !group.trim().is_empty(),
            "{name}: concurrency group must not be empty"
        );
    }
}

/// AC-1: a new run for the same PR cancels the in-flight one. The group is
/// keyed by workflow and PR number, so an update-branch `synchronize` replaces
/// the previous run instead of queueing beside it, and one PR's Test run never
/// cancels another PR's Test run or its own Build run.
#[test]
fn pull_request_workflows_cancel_the_superseded_run_of_the_same_pr() {
    let mut checked = 0;
    for (name, doc) in workflows() {
        if !triggered_by_pull_request(&doc) {
            continue;
        }
        checked += 1;
        let block = concurrency(&name, &doc);
        let group = block.get("group").and_then(Value::as_str).unwrap_or("");
        assert!(
            group.contains("github.workflow"),
            "{name}: concurrency group must be keyed by the workflow, got {group:?}"
        );
        assert!(
            group.contains("github.event.pull_request.number"),
            "{name}: concurrency group must be keyed by the PR number, got {group:?}"
        );
        assert!(
            cancel_in_progress(&name, &block),
            "{name}: pull request runs must set cancel-in-progress: true"
        );
    }
    assert!(checked >= 5, "expected the PR workflows (test, build, lint, auto-merge, pr-source-check), found {checked}");
}

/// A release run creates a tag and uploads assets; cancelling it half-way
/// leaves a partial release. Its group only serializes concurrent runs.
#[test]
fn release_workflow_never_cancels_an_in_flight_release() {
    let doc = workflow(RELEASE_WORKFLOW);
    let block = concurrency(RELEASE_WORKFLOW, &doc);
    assert!(
        !cancel_in_progress(RELEASE_WORKFLOW, &block),
        "{RELEASE_WORKFLOW}: cancel-in-progress must stay false"
    );
}

/// AC-2: a `changes` job classifies the PR once and exposes `docs_only`; the
/// heavy non-required jobs depend on it and skip when the PR only touches
/// documentation. The condition is written with `!cancelled()` so a failed
/// classification (for example an API error) falls open to running the jobs,
/// never to silently skipping them.
#[test]
fn docs_only_changes_skip_the_heavy_windows_and_webview_jobs() {
    let doc = workflow(TEST_WORKFLOW);
    let changes = job(&doc, CHANGES_JOB);
    let output = changes
        .get("outputs")
        .and_then(|outputs| outputs.get("docs_only"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            panic!("{TEST_WORKFLOW}: `{CHANGES_JOB}` must expose the `docs_only` output")
        });
    assert!(
        output.contains("steps.") && output.contains("outputs.docs_only"),
        "{TEST_WORKFLOW}: `docs_only` must be wired from a step output, got {output:?}"
    );

    for id in DOCS_SKIPPABLE_JOBS {
        let gated = job(&doc, id);
        assert!(
            needs(gated).iter().any(|n| n == CHANGES_JOB),
            "{TEST_WORKFLOW}: `{id}` must declare `needs: {CHANGES_JOB}`"
        );
        let condition = condition(gated);
        assert!(
            condition.contains(&format!("{DOCS_ONLY_OUTPUT} != 'true'")),
            "{TEST_WORKFLOW}: `{id}` must skip when `{DOCS_ONLY_OUTPUT}` is 'true', got {condition:?}"
        );
        assert!(
            condition.contains("!cancelled()"),
            "{TEST_WORKFLOW}: `{id}` must fall open with `!cancelled()` when classification fails, got {condition:?}"
        );
    }
}

/// AC-2: the path filter must not change what branch protection gates on.
/// Every required check still exists under its protected name, and none of
/// them depends on or conditions on the `changes` job.
#[test]
fn required_checks_are_never_gated_by_the_path_filter() {
    let mut seen = Vec::new();
    for (file, doc) in workflows() {
        for (id, body) in jobs(&doc) {
            let id = id.as_str().expect("job ids are strings");
            let display = body.get("name").and_then(Value::as_str).unwrap_or(id);
            if !REQUIRED_CHECKS.contains(&display) {
                continue;
            }
            seen.push(display.to_string());
            assert!(
                !needs(body).iter().any(|n| n == CHANGES_JOB),
                "{file}: required check `{display}` (job `{id}`) must not depend on `{CHANGES_JOB}`"
            );
            assert!(
                !condition(body).contains(CHANGES_JOB),
                "{file}: required check `{display}` (job `{id}`) must not condition on `{CHANGES_JOB}`"
            );
        }
    }
    let mut missing: Vec<&str> = REQUIRED_CHECKS
        .iter()
        .copied()
        .filter(|required| !seen.iter().any(|s| s == required))
        .collect();
    missing.sort();
    assert!(
        missing.is_empty(),
        "every develop required status check must still be produced by a workflow job; missing: {missing:?}"
    );
}
