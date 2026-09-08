//! SPEC #3200 — OPT-IN live smoke test against REAL GitHub (`#[ignore]`).
//!
//! Unlike the mock-gh E2E, this runs the real `gwt_git` / gate functions against
//! an actual GitHub PR and performs a real `gh pr merge`. It is `#[ignore]` so it
//! never runs in normal CI (needs network + `gh` auth + a prepared throwaway
//! repo). Run it explicitly:
//!
//! ```text
//! GWT_E2E_SLUG=owner/repo GWT_E2E_REPO=/path/to/clone \
//!   GWT_E2E_PR=2 GWT_E2E_SHA=<head sha> GWT_E2E_BRANCH=work/issue-2 \
//!   cargo test -p gwt --test autonomous_real_gh_smoke -- --ignored --nocapture
//! ```
//!
//! It verifies, end-to-end against real GitHub: branch-protection parse →
//! eligibility → gate (real CI rollup + reviewed-SHA binding) → real merge →
//! layer-4 (merged head SHA == reviewed SHA) → completion.

use std::path::PathBuf;

use gwt::issue_monitor_authz::merged_sha_matches_reviewed;
use gwt::issue_monitor_gate::{
    classify_acceptance_criteria, classify_ci_rollup, evaluate_autonomous_gate,
    route_autonomous_gate, GateAction, GateDecision,
};
use gwt_core::process::hidden_command;
use gwt_git::branch_protection::fetch_branch_protection;
use gwt_git::pr_status::{
    fetch_open_pr_number_for_branch, fetch_pr_head_sha, fetch_pr_status_check_rollup,
    merge_pr_auto, parse_pr_merge_commit_sha,
};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing env {name}"))
}

#[test]
#[ignore = "live: needs real GitHub repo + gh auth; run with --ignored"]
fn live_autonomous_merge_against_real_github() {
    let slug = env("GWT_E2E_SLUG");
    let repo = PathBuf::from(env("GWT_E2E_REPO"));
    let pr: u64 = env("GWT_E2E_PR").parse().expect("GWT_E2E_PR is a number");
    let reviewed = env("GWT_E2E_SHA");
    let branch = env("GWT_E2E_BRANCH");
    let body = "## Acceptance Criteria\n- [ ] AC-1: change2 present\n";

    // 1) Real branch protection → Verified (after the personal-repo restrictions fix).
    let protection = fetch_branch_protection(&slug, "main");
    println!("branch_protection = {protection:?}");
    assert!(
        protection.is_verified(),
        "real branch protection must verify"
    );

    // 2) Real PR discovery + head SHA binding.
    assert_eq!(
        fetch_open_pr_number_for_branch(&repo, &branch),
        Some(pr),
        "open PR for branch",
    );
    let head = fetch_pr_head_sha(&repo, pr).expect("PR head sha");
    assert_eq!(head, reviewed, "head SHA == reviewed SHA (no advance)");

    // 3) Real CI rollup → classify against the branch-protection required checks.
    let rollup = fetch_pr_status_check_rollup(&repo, pr);
    println!("rollup = {rollup}");
    let required: Vec<String> = match &protection {
        gwt_git::branch_protection::BranchProtectionStatus::Verified { required_checks } => {
            required_checks.clone()
        }
        _ => unreachable!(),
    };
    let ci = classify_ci_rollup(&rollup, &required);
    println!("ci = {ci:?}");

    // 4) Strong gate with real inputs ⇒ Pass / Deliver.
    let mut monitor = gwt::IssueMonitorState::with_prefs(
        gwt::IssueMonitorConfig::default(),
        gwt::IssueMonitorPrefs {
            autonomous_mode: true,
            ..gwt::IssueMonitorPrefs::default()
        },
    );
    monitor.capture_acceptance_snapshot(pr, classify_acceptance_criteria(body).snapshot());
    monitor.begin_review(pr, pr, &reviewed);
    monitor.record_review_verdict(pr, true);
    let inputs = monitor
        .autonomous_gate_inputs(pr, protection, &rollup, &head, body)
        .expect("gate ready");
    assert_eq!(
        evaluate_autonomous_gate(&inputs),
        GateDecision::Pass,
        "real gate Pass"
    );
    assert_eq!(
        route_autonomous_gate(&inputs),
        GateAction::Deliver,
        "route Deliver"
    );

    // 5) REAL merge via the production function, bound to the reviewed head SHA.
    assert!(
        merge_pr_auto(&repo, pr, &reviewed),
        "real merge_pr_auto armed/succeeded"
    );

    // 6) Poll for merge completion, then layer-4: merged head SHA == reviewed SHA.
    let mut merged = false;
    for _ in 0..30 {
        // `gh pr view --json mergeCommit` populated ⇒ merged.
        let view = hidden_command("gh")
            .args([
                "pr",
                "view",
                &pr.to_string(),
                "--repo",
                &slug,
                "--json",
                "mergeCommit",
            ])
            .output()
            .expect("gh pr view");
        let stdout = String::from_utf8_lossy(&view.stdout);
        if parse_pr_merge_commit_sha(&stdout).is_some() {
            merged = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    assert!(merged, "PR actually merged on real GitHub");

    let merged_head = fetch_pr_head_sha(&repo, pr).expect("post-merge head sha");
    println!("reviewed={reviewed} merged_head={merged_head}");
    assert!(
        merged_sha_matches_reviewed(&reviewed, &merged_head),
        "layer-4: the SHA that merged equals the reviewed SHA",
    );
    println!("LIVE AUTONOMOUS MERGE VERIFIED against {slug} PR #{pr}");
}
