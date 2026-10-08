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
//! `cancel-in-progress` so a product push supersedes the previous run, and a
//! path filter that keeps docs-only changes off the heavy Windows jobs and the
//! WebView E2E job. These tests pin both, plus the property that the filter
//! never touches a required status check.
//!
//! Issue #4872 removes the cause rather than containing it: develop lands
//! through a GitHub merge queue, so a merge no longer sends every other PR
//! back through update-branch. The queue only works if the same workflows
//! report under `merge_group`, which the tests below pin as well.
//! Issue #5059 moves Test cancellation to jobs after a Git comparison: base
//! synchronization must not discard an in-flight long measurement.

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
    // Issue #4522. Added to the protection contexts once lint.yml's
    // `Clippy (macOS)` job had landed on develop — adding a context before
    // the job exists strands every open PR on a check that never reports.
    "Clippy (macOS)",
];

/// Heavy, non-required jobs in `test.yml` that a documentation edit cannot
/// affect: the Windows agent launch E2E job and the WebView E2E job. The
/// three-pass Windows default-parallel job used to be listed here; Issue #4134
/// AC-1 moved it out of the pull-request path into `nightly.yml`, where a
/// per-PR path filter has nothing to skip.
const DOCS_SKIPPABLE_JOBS: &[&str] = &["test-windows-agent-launch-e2e", "test-frontend"];

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

/// AC-1 / Issue #5059 AC-2: product pushes still supersede the same PR's work.
/// Test classifies the source first; other workflows retain immediate per-PR
/// cancellation. Workflow/job keys keep PRs and workloads independent.
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
        if name == TEST_WORKFLOW {
            assert!(
                group.contains("github.run_id"),
                "Test must not cancel an entire run before classifying base synchronization"
            );
            for (id, body) in jobs(&doc) {
                let id = id.as_str().expect("job ids are strings");
                let policy = body
                    .get("concurrency")
                    .expect("Test jobs must bound their own workload");
                let key = policy.get("group").and_then(Value::as_str).unwrap_or("");
                assert!(
                    key.contains("github.workflow")
                        && key.contains("github.event.pull_request.number")
                );
                assert!(key.ends_with(id), "job groups must be independent: {id}");
                if matches!(id, "source-sync" | "changes") {
                    assert!(
                        cancel_in_progress(id, policy),
                        "short classifiers remain replaceable"
                    );
                } else {
                    assert!(needs(body).iter().any(|need| need == "source-sync"));
                    assert_eq!(
                        policy.get("cancel-in-progress").and_then(Value::as_str),
                        Some("${{ needs.source-sync.outputs.base_only == 'false' }}"),
                        "{id}: actual pushes cancel running work; proven base sync and unknown history preserve it"
                    );
                }
            }
        }
    }
    assert!(checked >= 5, "expected the PR workflows (test, build, lint, auto-merge, pr-source-check), found {checked}");
}

/// Whether the workflow's `pull_request` trigger targets `develop`.
fn gates_develop_pull_requests(doc: &Value) -> bool {
    triggers(doc)
        .get("pull_request")
        .and_then(|event| event.get("branches"))
        .and_then(Value::as_sequence)
        .is_some_and(|branches| branches.iter().any(|b| b.as_str() == Some("develop")))
}

/// Issue #5169 AC-1: strict=false permits combinations that were never tested
/// together before merge. Run the same gates on the resulting develop tree.
#[test]
fn required_ci_workflows_run_on_develop_pushes() {
    for name in ["test.yml", "lint.yml", "build.yml"] {
        let doc = workflow(name);
        let branches = triggers(&doc)
            .get("push")
            .and_then(|event| event.get("branches"))
            .and_then(Value::as_sequence)
            .unwrap_or_else(|| panic!("{name}: develop pushes must run post-merge CI"));
        assert_eq!(branches, &vec![Value::String("develop".into())]);
        for (id, body) in jobs(&doc) {
            let condition = condition(body);
            assert!(
                !condition.contains("github.event_name") || condition.contains("push"),
                "{name}/{id:?}: an event condition must not skip post-merge CI"
            );
        }
    }
}

/// Issue #5169 AC-1: push runs have no PR files API; classify their own diff.
#[test]
fn changed_files_on_develop_push_use_the_push_commit_range() {
    let doc = workflow(TEST_WORKFLOW);
    let steps = job(&doc, CHANGES_JOB)["steps"].as_sequence().unwrap();
    let classify = steps
        .iter()
        .find(|step| step["id"].as_str() == Some("classify"))
        .unwrap();
    let env = serde_yaml::to_string(&classify["env"]).unwrap();
    assert!(env.contains("github.event.before"));
    assert!(env.contains("github.event.after"));
    let script = classify["run"].as_str().unwrap();
    assert!(
        script.contains("push") && script.contains("PUSH_BASE") && script.contains("PUSH_HEAD")
    );
}

#[test]
fn post_merge_runs_do_not_cancel_other_develop_trees_or_their_coverage() {
    for name in ["test.yml", "lint.yml", "build.yml", "coverage.yml"] {
        let doc = workflow(name);
        let policy = concurrency(name, &doc);
        let group = policy["group"].as_str().unwrap();
        assert!(
            group.contains("github.sha") || group.contains("github.run_id"),
            "{name}: push trees need independent groups"
        );
        if name == TEST_WORKFLOW {
            for (_, body) in jobs(&doc) {
                let group = body["concurrency"]["group"].as_str().unwrap();
                assert!(
                    group.contains("github.event_name == 'push'") && group.contains("github.sha")
                );
            }
        }
    }
}

/// Issue #5169 AC-2: a develop failure must have a durable notification target.
#[test]
fn post_merge_failures_are_reported_to_a_bug_issue() {
    let doc = workflow("post-merge-ci.yml");
    // The default branch is main. A workflow_run listener added to develop
    // cannot report anything until release, so call from the tested tree.
    assert!(triggers(&doc).get("workflow_call").is_some());
    for name in ["test.yml", "lint.yml", "build.yml"] {
        let caller = workflow(name);
        let report = job(&caller, "report-post-merge-failure");
        let guard = condition(report);
        assert!(guard.contains("github.event_name == 'push'") && guard.contains("failure()"));
        assert_eq!(
            report["uses"].as_str(),
            Some("./.github/workflows/post-merge-ci.yml")
        );
        for (id, _) in jobs(&caller) {
            let id = id.as_str().unwrap();
            if id != "report-post-merge-failure" {
                assert!(
                    needs(report).iter().any(|dependency| dependency == id),
                    "{name}: {id} failures must reach the reporter"
                );
            }
        }
    }
    let report = job(&doc, "report-failure");
    assert_eq!(report["permissions"]["issues"].as_str(), Some("write"));
    let script = serde_yaml::to_string(&report["steps"]).unwrap();
    assert!(script.contains("gh issue create") && script.contains("--label bug"));
    assert!(script.contains("gh issue comment") && script.contains("RUN_URL"));
}

/// Issue #4872: develop lands through a GitHub merge queue, which re-tests each
/// pull request on the latest base under a `merge_group` event. A required
/// check that no workflow reports for that event stays pending forever and
/// nothing lands, so every workflow that gates a develop pull request must run
/// for the queue too — and no job in it may be conditioned on the
/// `pull_request` event alone, because a job skipped that way reports Success
/// and the queue would land a change the check never looked at.
#[test]
fn develop_pull_request_workflows_also_run_for_the_merge_queue() {
    let mut checked = 0;
    for (name, doc) in workflows() {
        if !gates_develop_pull_requests(&doc) {
            continue;
        }
        checked += 1;
        assert!(
            triggers(&doc).get("merge_group").is_some(),
            "{name}: a workflow that gates develop pull requests must also trigger on `merge_group`"
        );
        for (id, body) in jobs(&doc) {
            let id = id.as_str().expect("job ids are strings");
            if id == "report-post-merge-failure" {
                // Notifications are for failed push runs, not merge-queue gates.
                continue;
            }
            let condition = condition(body);
            assert!(
                !condition.contains("github.event_name") || condition.contains("merge_group"),
                "{name}: job `{id}` is limited by event name and skips the merge queue, got {condition:?}"
            );
        }
    }
    assert!(
        checked >= 3,
        "expected the develop PR workflows (test, build, lint), found {checked}"
    );
}

/// Issue #4872: a merge group has no pull request number, so the classifier
/// cannot ask the pulls API which files changed. Without its own source the
/// step fails on every queue run and the jobs that read its outputs stop
/// meaning what they mean on a pull request — the flake job would never run in
/// the queue at all. The group's files come from its diff against the commit
/// it was built on.
#[test]
fn the_changed_file_classifier_reads_a_merge_group_from_its_own_diff() {
    let doc = workflow(TEST_WORKFLOW);
    let step = job(&doc, CHANGES_JOB)
        .get("steps")
        .and_then(Value::as_sequence)
        .and_then(|steps| {
            steps
                .iter()
                .find(|step| step.get("id").and_then(Value::as_str) == Some("classify"))
        })
        .unwrap_or_else(|| {
            panic!("{TEST_WORKFLOW}: `{CHANGES_JOB}` must keep the `classify` step")
        });
    let env = serde_yaml::to_string(step.get("env").unwrap_or(&Value::Null)).unwrap_or_default();
    for sha in [
        "github.event.merge_group.base_sha",
        "github.event.merge_group.head_sha",
    ] {
        assert!(
            env.contains(sha),
            "{TEST_WORKFLOW}: the `classify` step must receive `{sha}`"
        );
    }
    let script = step.get("run").and_then(Value::as_str).unwrap_or("");
    assert!(
        script.contains("merge_group") && script.contains("git diff --name-only"),
        "{TEST_WORKFLOW}: the `classify` step must diff the merge group instead of asking for a PR's files"
    );
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
