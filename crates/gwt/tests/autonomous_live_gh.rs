//! SPEC #3200 T-003/T-100 — live-integration E2E: the autonomous merge loop
//! EXECUTES the scan/proposal half through the real `advance_autonomous_in_flight`
//! orchestration, then exercises the same real `gwt_git` readback/arm adapter used
//! by the daemon's serialized executor against a SCRIPTED MOCK `gh` on PATH.
//! No real GitHub is touched. The daemon transaction/fence contract has focused
//! coverage in `cli::daemon::server`; this test keeps the cross-crate subprocess
//! pipeline (spawn → gh → parse → gate → proposal → arm) live and observable.
//!
//! Both scenarios live in ONE test: PATH + mock env are process-global, so a
//! single sequential test avoids cross-thread env races.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use gwt::issue_monitor_worker::try_advance_autonomous_in_flight;
use gwt::{
    AutonomousPhase, IssueMonitorConfig, IssueMonitorEffectPayload, IssueMonitorEffectState,
    IssueMonitorIssue, IssueMonitorIssueState, IssueMonitorPrefs, IssueMonitorState,
    MonitorInboxState,
};

const BODY: &str = "## Acceptance Criteria\n- [ ] AC-1: returns 200\n";
const SHA: &str = "abc123";

/// A mock `gh` answering exactly the calls the autonomous loop makes, recording
/// the irreversible `pr merge --auto` invocation to `$GWT_MOCK_GH_LOG`.
///
/// Live-verified SHA semantics (real GitHub, squash merge): `headRefOid` is the
/// head tip that was merged (== reviewed SHA when HEAD did not advance) and is
/// what the layer-4 check compares; `mergeCommit.oid` is a NEW commit (presence
/// = merged), so it is intentionally a different value here.
const MOCK_GH: &str = r#"#!/bin/sh
all="$*"
case "$all" in
  *api*"/protection"*)
    echo '{"required_status_checks":{"contexts":["build"]},"restrictions":null,"allow_force_pushes":{"enabled":false}}' ;;
  *"pr view"*state,headRefOid,autoMergeRequest,mergeCommit*)
    echo '{"state":"OPEN","headRefOid":"abc123","autoMergeRequest":null,"mergeCommit":null}' ;;
  *"pr view"*headRefOid*)         echo '{"headRefOid":"abc123"}' ;;
  *"pr view"*statusCheckRollup*)  echo '{"statusCheckRollup":[{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}]}' ;;
  *"pr view"*mergeCommit*)        echo '{"mergeCommit":{"oid":"squashcommit999"}}' ;;
  *"pr diff"*)                    echo 'diff --git a/x b/x' ;;
  *"pr list"*)                   echo '[{"number":7}]' ;;
  *"pr merge"*--auto*)            echo "MERGE $all" >> "$GWT_MOCK_GH_LOG" ;;
  *) : ;;
esac
exit 0
"#;

fn auto_issue() -> IssueMonitorIssue {
    IssueMonitorIssue {
        number: 42,
        title: "Issue 42".to_string(),
        labels: vec!["auto-merge".to_string()],
        state: IssueMonitorIssueState::Open,
        body: Some(BODY.to_string()),
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
        updated_at: Some("2026-06-29T00:00:00Z".to_string()),
    }
}

fn reviewed_monitor() -> IssueMonitorState {
    let mut monitor = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        IssueMonitorPrefs {
            enabled: true,
            autonomous_mode: true,
            ..IssueMonitorPrefs::default()
        },
    );
    gwt::scan_issue_monitor_candidates(&mut monitor, &[auto_issue()], "2026-06-29T00:00:00Z");
    monitor.capture_acceptance_snapshot(
        42,
        gwt::issue_monitor_gate::classify_acceptance_criteria(BODY).snapshot(),
    );
    monitor.begin_review(42, 7, SHA);
    monitor.record_review_verdict(42, true);
    monitor
}

fn init_repo_with_default_branch(repo: &Path) {
    let init = gwt_core::process::hidden_command("git")
        .args(["init", "-q", "-b", "main"])
        .arg(repo)
        .output()
        .expect("git init");
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let origin_head = gwt_core::process::hidden_command("git")
        .args([
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ])
        .current_dir(repo)
        .output()
        .expect("set origin HEAD");
    assert!(
        origin_head.status.success(),
        "git symbolic-ref failed: {}",
        String::from_utf8_lossy(&origin_head.stderr)
    );
}

#[test]
fn autonomous_merge_pipeline_executes_through_mock_gh() {
    let tmp = std::env::temp_dir().join(format!("gwt-mockgh-{}", std::process::id()));
    let bin = tmp.join("bin");
    fs::create_dir_all(&bin).expect("mkdir mock bin");
    let gh = bin.join("gh");
    fs::write(&gh, MOCK_GH).expect("write mock gh");
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).expect("chmod mock gh");
    let merge_log = tmp.join("merge.log");
    let repo = tmp.join("repo");
    init_repo_with_default_branch(&repo);

    let orig_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{}", bin.display(), orig_path));
    std::env::set_var("GWT_MOCK_GH_LOG", &merge_log);

    let now = "2026-06-29T00:10:00Z";
    let issues = [auto_issue()];

    // Full pass → the real merge_pr_auto executes against the (mock) gh
    // subprocess → layer-4 (reviewed == headRefOid) holds → completion.
    let _ = fs::remove_file(&merge_log);
    let mut monitor = reviewed_monitor();

    // Tick 1: Reviewing → real fetchers (mock gh) → real gate → durable arm
    // proposal. The scan itself has no authority to invoke the remote mutation.
    try_advance_autonomous_in_flight(&mut monitor, &issues, "test/repo", &repo, b"secret", now)
        .expect("gate scan succeeds");
    assert_eq!(
        monitor.autonomous_record(42).map(|r| r.phase),
        Some(AutonomousPhase::Reviewing),
        "gate pass remains Reviewing until the exact executor result commits",
    );
    assert!(
        fs::read_to_string(&merge_log)
            .unwrap_or_default()
            .is_empty(),
        "the proposal-only scan must not invoke the remote merge adapter",
    );
    let effect = monitor
        .pending_effects()
        .first()
        .cloned()
        .expect("gate pass prepares an arm effect");
    assert_eq!(effect.state, IssueMonitorEffectState::Prepared);
    let IssueMonitorEffectPayload::ArmAutoMerge {
        issue_number,
        pr_number,
        reviewed_sha,
    } = &effect.payload
    else {
        panic!("gate pass must prepare ArmAutoMerge");
    };
    assert_eq!(
        (*issue_number, *pr_number, reviewed_sha.as_str()),
        (42, 7, SHA)
    );
    assert_eq!(effect.authority_epoch, monitor.effect_authority_epoch());

    // The daemon persists Prepared, fences the exact tuple as Attempting, and
    // only then calls this adapter from its serialized executor. Exercise that
    // real subprocess boundary here, then model the already-focused exact
    // result commit before allowing Delivering.
    let key = effect.attempt_key();
    assert!(monitor.mark_pending_effect_attempting(&key));
    let remote = gwt_git::pr_status::fetch_pr_auto_merge_remote_state(&repo, 7)
        .expect("mock gh returns an authoritative open PR state");
    let outcome = gwt_git::pr_status::arm_pr_auto_merge(&repo, 7, SHA, &remote);
    assert!(outcome.is_success(), "arm outcome: {outcome:?}");
    assert!(monitor.complete_pending_effect(&key).is_some());
    monitor.begin_delivering(42);
    monitor.record_auto_merge_armed(42);
    assert_eq!(
        monitor.autonomous_record(42).map(|r| r.phase),
        Some(AutonomousPhase::Delivering),
        "only the committed executor success enters Delivering",
    );
    let log = fs::read_to_string(&merge_log).unwrap_or_default();
    assert!(
        log.contains("MERGE") && log.contains("pr merge"),
        "the real arm adapter invoked `gh pr merge --auto` (log={log:?})",
    );

    // Tick 2: Delivering → real merge-commit fetch (merged) + headRefOid==reviewed ⇒ done.
    try_advance_autonomous_in_flight(&mut monitor, &issues, "test/repo", &repo, b"secret", now)
        .expect("delivery scan succeeds");
    assert!(
        monitor.autonomous_record(42).is_none(),
        "merged head (headRefOid) == reviewed_sha ⇒ record cleared (completion)",
    );
    assert_eq!(
        monitor.inbox_item(42).map(|i| i.state),
        Some(MonitorInboxState::Merged),
    );

    cleanup(&tmp, &orig_path);
}

fn cleanup(tmp: &Path, orig_path: &str) {
    std::env::set_var("PATH", orig_path);
    std::env::remove_var("GWT_MOCK_GH_LOG");
    let _ = fs::remove_dir_all(tmp);
}
