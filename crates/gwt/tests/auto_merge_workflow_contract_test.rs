//! Contract tests for `.github/workflows/auto-merge.yml` (Issue #7700).
//!
//! The workflow armed auto-merge on every akiojin PR at open / synchronize.
//! For a PR that gwt Issues sends to independent review (head
//! `work/issue-N`, Issue N labeled `auto-merge`) that arm preceded the review:
//! PR #7695 was armed at 02:05:13Z, its review started at 02:08Z, required CI
//! merged it at 02:25:04Z and the `fail` verdict arrived at 02:25:32Z
//! (Issue #7697). The hold gwt Issues applies itself is a per-scan disarm
//! after the fact, so the workflow must not arm those PRs in the first place;
//! gwt Issues arms them once a pass verdict is recorded.
//!
//! The step's script is executed against a stub `gh`, so these tests pin what
//! the workflow does rather than how its text reads.

use gwt_core::process::{resolved_command, ProcessPlanRequest};
use serde_yaml::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

const WORKFLOW: &str = ".github/workflows/auto-merge.yml";

/// Stub `gh`: logs every invocation, answers `issue view` from the
/// `STUB_ISSUE_*` variables, and accepts everything else.
const GH_STUB: &str = r#"#!/bin/sh
echo "$*" >> "$STUB_GH_LOG"
if [ "$1" = "issue" ]; then
  if [ "$STUB_ISSUE_EXIT" != "0" ]; then
    echo "HTTP 502: Bad Gateway" >&2
    exit "$STUB_ISSUE_EXIT"
  fi
  printf '%s' "$STUB_ISSUE_LABELS"
fi
"#;

struct Run {
    succeeded: bool,
    gh_calls: Vec<String>,
}

impl Run {
    fn armed(&self) -> bool {
        self.gh_calls
            .iter()
            .any(|call| call.starts_with("pr merge"))
    }
}

fn arm_script() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(WORKFLOW);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let workflow: Value = serde_yaml::from_str(&text).expect("auto-merge.yml must parse");
    let steps = workflow["jobs"]["auto-merge"]["steps"]
        .as_sequence()
        .expect("auto-merge job must declare steps");
    assert_eq!(steps.len(), 1, "the arm decision must stay in one step");
    steps[0]["run"]
        .as_str()
        .expect("the arm step must be a run script")
        .to_string()
}

/// Runs the arm step the way the runner does (`bash -e`) for a PR whose head
/// is `head_ref`, with Issue reads answered by `issue_labels`
/// (`Err(code)` = the read fails with that exit code).
fn run_arm_step(head_ref: &str, issue_labels: Result<&str, i32>) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    let gh = dir.path().join("gh");
    fs::write(&gh, GH_STUB).expect("write gh stub");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("chmod gh stub");
    let script = dir.path().join("step.sh");
    fs::write(&script, arm_script()).expect("write step script");
    let log = dir.path().join("gh.log");

    let path = format!(
        "{}:{}",
        dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let status = resolved_command(ProcessPlanRequest::new("bash"))
        .expect("resolve bash")
        .arg("-e")
        .arg(&script)
        .env("PATH", path)
        .env("GH_TOKEN", "stub-token")
        .env("PR_NUMBER", "7")
        .env("REPO", "akiojin/gwt")
        .env("HEAD_REF", head_ref)
        .env("BASE_REF", "develop")
        .env("STUB_GH_LOG", &log)
        .env(
            "STUB_ISSUE_EXIT",
            issue_labels.err().unwrap_or(0).to_string(),
        )
        .env("STUB_ISSUE_LABELS", issue_labels.unwrap_or_default())
        .status()
        .expect("run the arm step under bash");

    Run {
        succeeded: status.success(),
        gh_calls: fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect(),
    }
}

/// AC-1: gwt Issues arms a review-target PR after the pass verdict; the
/// workflow leaves it unarmed and the job still succeeds.
#[test]
fn review_target_pr_is_left_for_issues_to_arm() {
    for labels in ["auto-merge\n", "bug\nAuto-Merge\n"] {
        let run = run_arm_step("work/issue-42", Ok(labels));
        assert!(
            !run.armed(),
            "labels {labels:?}: armed before the verdict: {:?}",
            run.gh_calls
        );
        assert!(
            run.succeeded,
            "labels {labels:?}: skipping the arm is not a failure"
        );
        assert!(
            run.gh_calls
                .iter()
                .any(|call| call.starts_with("issue view 42 ")),
            "the decision must read Issue #42: {:?}",
            run.gh_calls
        );
    }
}

/// AC-2: an unreadable Issue cannot be told apart from a review target, so the
/// step fails closed — no arm, and a red job instead of a silent skip.
#[test]
fn unreadable_issue_fails_the_job_without_arming() {
    let run = run_arm_step("work/issue-42", Err(1));
    assert!(!run.armed(), "armed without the Issue: {:?}", run.gh_calls);
    assert!(!run.succeeded, "an Issue read failure must fail the job");
}

/// The guard is scoped to review targets: every other PR keeps the arm it had.
#[test]
fn other_prs_are_still_armed() {
    for (head_ref, labels) in [
        ("work/issue-42", "bug\n"),
        ("work/issue-42", ""),
        ("develop", "auto-merge\n"),
        ("dependabot/cargo/serde-1.0.0", "auto-merge\n"),
    ] {
        let run = run_arm_step(head_ref, Ok(labels));
        assert!(
            run.succeeded,
            "{head_ref} with labels {labels:?} must succeed"
        );
        assert_eq!(
            run.gh_calls.last().map(String::as_str),
            Some("pr merge 7 --auto --merge --repo akiojin/gwt"),
            "{head_ref} with labels {labels:?}"
        );
    }
}
