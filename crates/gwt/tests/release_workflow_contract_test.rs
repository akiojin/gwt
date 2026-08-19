//! Contract tests for the release preparation workflow (Issue #3428).
//!
//! `Prepare Release` is the only supported release entrypoint, and every run
//! writes to a protected branch (`develop`) and opens the `develop -> main`
//! Release PR. Both actions need credentials the default `GITHUB_TOKEN` does
//! not have, so the token wiring is a contract — not an implementation
//! detail — and is pinned here.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("gwt crate must be nested under crates/")
        .to_path_buf()
}

fn prepare_release_workflow() -> String {
    let path = repo_root().join(".github/workflows/prepare-release.yml");
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The bump commit lands on `develop`, which requires a pull request and 9
/// status checks. `GITHUB_TOKEN` cannot bypass that, so the push is rejected
/// with `GH006: Protected branch update failed` after the whole bump has
/// already been computed.
#[test]
fn prepare_release_pushes_to_protected_develop_with_bypass_credentials() {
    let workflow = prepare_release_workflow();

    assert!(
        !workflow.contains("token: ${{ secrets.GITHUB_TOKEN }}"),
        "the develop checkout must not carry GITHUB_TOKEN push credentials; \
         pushing the bump commit to protected develop needs the bypass PAT"
    );
    assert!(
        workflow.contains("token: ${{ secrets.PERSONAL_ACCESS_TOKEN }}"),
        "the develop checkout must use PERSONAL_ACCESS_TOKEN so the bump push \
         is not rejected by branch protection"
    );
}

/// A pull request opened with `GITHUB_TOKEN` does not start workflow runs, so
/// its 9 required checks stay pending forever and the Release PR can never
/// merge. The PR must be opened with the same PAT.
#[test]
fn release_pull_request_is_opened_with_credentials_that_start_required_checks() {
    let workflow = prepare_release_workflow();
    let pr_step = workflow
        .split("- name: Create or update Release PR")
        .nth(1)
        .expect("workflow must keep the Release PR step");

    assert!(
        pr_step.contains("GH_TOKEN: ${{ secrets.PERSONAL_ACCESS_TOKEN }}"),
        "the Release PR must be opened with PERSONAL_ACCESS_TOKEN; a PR opened \
         by GITHUB_TOKEN never triggers its own required checks"
    );
}

/// `scripts/release_issue_refs.py` classifies every ref through `gh`, which
/// refuses to run inside Actions without a token. The script propagates that
/// failure, so an unauthenticated step fails the release run — after the bump
/// has already been pushed.
#[test]
fn closing_issue_collection_runs_gh_with_a_token() {
    let workflow = prepare_release_workflow();
    let step = workflow
        .split("- name: Collect closing issues")
        .nth(1)
        .and_then(|rest| rest.split("- name: ").next())
        .expect("workflow must keep the closing-issue step");

    assert!(
        step.contains("GH_TOKEN:"),
        "collecting closing issues shells out to `gh`, which needs a token in \
         Actions: {step}"
    );
}

/// The bump is expensive (toolchain install, `cargo set-version`, `cargo
/// update`, git-cliff). Discovering a missing secret only at push time throws
/// that work away and leaves an orphan local commit, so the run must refuse
/// before mutating anything.
#[test]
fn prepare_release_fails_fast_when_the_bypass_secret_is_missing() {
    let workflow = prepare_release_workflow();
    let preflight = workflow
        .split("- name: Apply version bump")
        .next()
        .expect("workflow must define steps before the version bump");

    assert!(
        preflight.contains("PERSONAL_ACCESS_TOKEN is not configured"),
        "the workflow must fail with an actionable message before the version \
         bump when the release PAT is missing"
    );
}
