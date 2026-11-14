//! Contract tests for the repository auto-merge opt-in (Issue #3644).
//!
//! Issue registration uses the `auto-merge` Issue label as the durable opt-in,
//! while pull requests targeting `develop` do not populate GitHub's
//! `closingIssuesReferences`. Managed work therefore resolves its owner from
//! the canonical `work/issue-N` branch name and reads the Issue label directly.

#![cfg(unix)]

use gwt_core::process::{resolved_command, ProcessPlanRequest};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;

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

#[derive(Debug)]
struct EligibilityRun {
    output: Output,
    values: HashMap<String, String>,
    gh_calls: String,
}

fn run_eligibility(author: &str, head: &str, base: &str, gh_mode: &str) -> EligibilityRun {
    run_eligibility_from(author, head, base, "akiojin/gwt", gh_mode)
}

fn run_eligibility_from(
    author: &str,
    head: &str,
    base: &str,
    head_repo: &str,
    gh_mode: &str,
) -> EligibilityRun {
    let dir = tempfile::tempdir().expect("temporary fake-gh directory");
    let fake_gh = dir.path().join("gh");
    fs::write(
        &fake_gh,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$GWT_FAKE_GH_LOG"
case "$GWT_FAKE_GH_MODE" in
  true) printf '%s\n' true ;;
  false) printf '%s\n' false ;;
  error) printf '%s\n' 'simulated issue lookup failure' >&2; exit 1 ;;
  *) printf '%s\n' "unexpected fake gh mode: $GWT_FAKE_GH_MODE" >&2; exit 2 ;;
esac
"#,
    )
    .expect("write fake gh");
    let mut permissions = fs::metadata(&fake_gh)
        .expect("fake gh metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_gh, permissions).expect("make fake gh executable");

    let github_output = dir.path().join("github-output");
    let gh_log = dir.path().join("gh.log");
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let command_path = std::env::join_paths(
        std::iter::once(dir.path().to_path_buf()).chain(std::env::split_paths(&inherited_path)),
    )
    .expect("compose fake-gh PATH");

    let mut command = resolved_command(
        ProcessPlanRequest::new("bash").arg(repo_root().join("scripts/auto-merge-eligibility.sh")),
    )
    .expect("resolve bash");
    let output = command
        .env("PATH", command_path)
        .env("GITHUB_OUTPUT", &github_output)
        .env("GWT_FAKE_GH_LOG", &gh_log)
        .env("GWT_FAKE_GH_MODE", gh_mode)
        .env("PR_AUTHOR", author)
        .env("HEAD_REF", head)
        .env("HEAD_REPO", head_repo)
        .env("BASE_REF", base)
        .env("REPO", "akiojin/gwt")
        .output()
        .expect("run auto-merge eligibility helper");

    let values = fs::read_to_string(&github_output)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    let gh_calls = fs::read_to_string(gh_log).unwrap_or_default();
    EligibilityRun {
        output,
        values,
        gh_calls,
    }
}

#[test]
fn managed_issue_branch_reads_the_issue_auto_merge_label() {
    let eligible = run_eligibility("akiojin", "work/issue-3701", "develop", "true");
    assert!(
        eligible.output.status.success(),
        "{}",
        String::from_utf8_lossy(&eligible.output.stderr)
    );
    assert_eq!(
        eligible.values.get("eligible").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        eligible.values.get("lookup_ok").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        eligible.values.get("issue_number").map(String::as_str),
        Some("3701")
    );
    assert!(
        eligible
            .gh_calls
            .contains("issue view 3701 --repo akiojin/gwt --json labels"),
        "the helper must read the Issue label directly: {}",
        eligible.gh_calls
    );
}

#[test]
fn managed_issue_without_auto_merge_label_is_an_explicit_opt_out() {
    let opted_out = run_eligibility("akiojin", "work/issue-3644", "develop", "false");
    assert!(opted_out.output.status.success());
    assert_eq!(
        opted_out.values.get("eligible").map(String::as_str),
        Some("false")
    );
    assert_eq!(
        opted_out.values.get("lookup_ok").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        opted_out.values.get("reason").map(String::as_str),
        Some("issue_missing_auto_merge_label")
    );
}

#[test]
fn issue_lookup_failure_is_fail_closed_and_distinguishable_from_opt_out() {
    let lookup_failure = run_eligibility("akiojin", "work/issue-3644", "develop", "error");
    assert!(
        lookup_failure.output.status.success(),
        "the workflow must receive outputs so it can disarm before failing: {}",
        String::from_utf8_lossy(&lookup_failure.output.stderr)
    );
    assert_eq!(
        lookup_failure.values.get("eligible").map(String::as_str),
        Some("false")
    );
    assert_eq!(
        lookup_failure.values.get("lookup_ok").map(String::as_str),
        Some("false")
    );
    assert_eq!(
        lookup_failure.values.get("reason").map(String::as_str),
        Some("issue_label_lookup_failed")
    );
}

#[test]
fn dependabot_and_release_prs_preserve_the_existing_auto_merge_paths() {
    for (author, head, base, source) in [
        (
            "dependabot[bot]",
            "dependabot/cargo/uuid",
            "develop",
            "dependabot",
        ),
        ("akiojin", "develop", "main", "release"),
    ] {
        let run = run_eligibility(author, head, base, "error");
        assert!(
            run.output.status.success(),
            "{}",
            String::from_utf8_lossy(&run.output.stderr)
        );
        assert_eq!(run.values.get("eligible").map(String::as_str), Some("true"));
        assert_eq!(run.values.get("source").map(String::as_str), Some(source));
        assert!(
            run.gh_calls.is_empty(),
            "{source} must not depend on an Issue lookup: {}",
            run.gh_calls
        );
    }
}

#[test]
fn unmanaged_owner_branch_fails_closed_without_guessing_an_issue() {
    let run = run_eligibility("akiojin", "feature/manual", "develop", "error");
    assert!(run.output.status.success());
    assert_eq!(
        run.values.get("eligible").map(String::as_str),
        Some("false")
    );
    assert_eq!(
        run.values.get("reason").map(String::as_str),
        Some("unmanaged_owner_branch")
    );
    assert!(run.gh_calls.is_empty());
}

#[test]
fn unauthorized_author_and_nonrelease_main_branch_fail_closed() {
    for (author, head, base, reason) in [
        (
            "someone-else",
            "work/issue-3701",
            "develop",
            "unauthorized_author",
        ),
        (
            "akiojin",
            "work/issue-3701",
            "main",
            "unsupported_base_branch",
        ),
    ] {
        let run = run_eligibility(author, head, base, "error");
        assert!(run.output.status.success());
        assert_eq!(
            run.values.get("eligible").map(String::as_str),
            Some("false")
        );
        assert_eq!(run.values.get("reason").map(String::as_str), Some(reason));
        assert!(
            run.gh_calls.is_empty(),
            "rejected requests must not probe arbitrary Issues: {}",
            run.gh_calls
        );
    }
}

#[test]
fn owner_fork_named_develop_is_not_a_release_pr() {
    let run = run_eligibility_from("akiojin", "develop", "main", "akiojin/gwt-fork", "error");
    assert!(run.output.status.success());
    assert_eq!(
        run.values.get("eligible").map(String::as_str),
        Some("false")
    );
    assert_eq!(
        run.values.get("reason").map(String::as_str),
        Some("foreign_head_repository")
    );
    assert!(run.gh_calls.is_empty());
}

#[test]
fn sweep_disarms_preexisting_opt_outs_without_touching_opted_in_prs() {
    let dir = tempfile::tempdir().expect("temporary fake-gh directory");
    let fake_gh = dir.path().join("gh");
    fs::write(
        &fake_gh,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$GWT_FAKE_GH_LOG"
if [ "$1 $2" = "pr list" ]; then
  printf '%s\n' '500|akiojin|work/issue-3644|akiojin/gwt|develop'
  printf '%s\n' '501|akiojin|work/issue-3701|akiojin/gwt|develop'
elif [ "$1 $2" = "issue view" ]; then
  if [ "$3" = "3701" ]; then printf '%s\n' true; else printf '%s\n' false; fi
elif [ "$1 $2" = "pr merge" ]; then
  exit 0
else
  printf '%s\n' "unexpected fake gh call: $*" >&2
  exit 2
fi
"#,
    )
    .expect("write fake gh");
    let mut permissions = fs::metadata(&fake_gh)
        .expect("fake gh metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_gh, permissions).expect("make fake gh executable");

    let gh_log = dir.path().join("gh.log");
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let command_path = std::env::join_paths(
        std::iter::once(dir.path().to_path_buf()).chain(std::env::split_paths(&inherited_path)),
    )
    .expect("compose fake-gh PATH");
    let mut command = resolved_command(
        ProcessPlanRequest::new("bash")
            .arg(repo_root().join("scripts/audit-auto-merge-opt-outs.sh")),
    )
    .expect("resolve bash");
    let output = command
        .env("PATH", command_path)
        .env("GWT_FAKE_GH_LOG", &gh_log)
        .env("REPO", "akiojin/gwt")
        .output()
        .expect("run open-PR auto-merge audit");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let calls = fs::read_to_string(gh_log).expect("read fake-gh calls");
    assert!(
        calls.contains("pr merge 500 --disable-auto --repo akiojin/gwt"),
        "the existing opt-out must be disarmed: {calls}"
    );
    assert!(
        !calls.contains("pr merge 501 --disable-auto"),
        "the opted-in PR must remain armed: {calls}"
    );
}

#[test]
fn workflow_disarms_opt_outs_and_audits_auto_merged_prs() {
    let workflow = read(".github/workflows/auto-merge.yml");
    let _: serde_yaml::Value =
        serde_yaml::from_str(&workflow).expect("auto-merge workflow must remain valid YAML");

    assert!(
        workflow.contains(
            "types: [opened, synchronize, reopened, ready_for_review, auto_merge_enabled, closed]"
        ),
        "closed and auto_merge_enabled events are required to audit and disarm opt-outs"
    );
    assert!(
        workflow.contains("github.event.action != 'closed'"),
        "the arming job must never run for the closed-event audit"
    );
    assert!(
        workflow.contains("scripts/auto-merge-eligibility.sh"),
        "arming and audit must share the Issue-label eligibility contract"
    );
    assert!(
        workflow.contains("steps.eligibility.outputs.eligible == 'false'")
            && workflow.contains("gh pr merge \"$PR_NUMBER\" --disable-auto"),
        "an opted-out PR must actively clear a stale auto-merge request"
    );
    assert!(
        workflow.contains("github.event.action != 'auto_merge_enabled'"),
        "an eligible auto_merge_enabled event must not recursively re-arm the PR"
    );
    assert!(
        workflow.contains("steps.eligibility.outputs.lookup_ok == 'false'"),
        "an Issue lookup failure must remain visible after the workflow disarms"
    );
    assert!(
        workflow.contains("autoMergeRequest") && workflow.contains("audit-opt-out"),
        "closed auto-merged PRs need an explicit opt-out audit"
    );
    assert!(
        workflow.contains("workflow_dispatch:")
            && workflow.contains("schedule:")
            && workflow.contains("scripts/audit-auto-merge-opt-outs.sh"),
        "a scheduled/manual sweep must disarm PRs armed before this policy was deployed"
    );
}
