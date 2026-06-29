//! SPEC #3200 T-003/T-100 — live-integration E2E: the autonomous merge loop
//! EXECUTES through the real production code path (the real `advance_autonomous_in_flight`
//! orchestration, the real `gwt_git::pr_status` / `branch_protection` fetchers that
//! actually spawn `gh`, and the real `merge_pr_auto`) against a SCRIPTED MOCK `gh`
//! on PATH. No real GitHub is touched — but unlike the unit tests, the actual
//! subprocess pipeline (spawn → gh → parse → gate → merge) runs, so the
//! irreversible merge call is genuinely exercised end-to-end and observed.
//!
//! Both scenarios live in ONE test: PATH + mock env are process-global, so a
//! single sequential test avoids cross-thread env races.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use gwt::issue_monitor_worker::advance_autonomous_in_flight;
use gwt::{
    AutonomousPhase, IssueMonitorConfig, IssueMonitorIssue, IssueMonitorIssueState,
    IssueMonitorPrefs, IssueMonitorState, MonitorInboxState,
};

const BODY: &str = "## Acceptance Criteria\n- [ ] AC-1: returns 200\n";
const SHA: &str = "abc123";

/// A mock `gh` answering exactly the calls the autonomous loop makes, recording
/// the irreversible `pr merge --auto` invocation to `$GWT_MOCK_GH_LOG`.
const MOCK_GH: &str = r#"#!/bin/sh
all="$*"
case "$all" in
  *api*"/protection"*)
    echo '{"required_status_checks":{"contexts":["build"]},"restrictions":{"users":[]},"allow_force_pushes":{"enabled":false}}' ;;
  *"pr view"*headRefOid*)         echo '{"headRefOid":"abc123"}' ;;
  *"pr view"*statusCheckRollup*)  echo '{"statusCheckRollup":[{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}]}' ;;
  *"pr view"*mergeCommit*)        echo "{\"mergeCommit\":{\"oid\":\"$GWT_MOCK_MERGE_OID\"}}" ;;
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
    }
}

fn reviewed_monitor() -> IssueMonitorState {
    let mut monitor = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        IssueMonitorPrefs {
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
    fs::create_dir_all(&repo).expect("mkdir repo");

    let orig_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{}", bin.display(), orig_path));
    std::env::set_var("GWT_MOCK_GH_LOG", &merge_log);

    let now = "2026-06-29T00:10:00Z";
    let issues = [auto_issue()];

    // --- Scenario A: full pass → the real merge executes → completion ---
    let _ = fs::remove_file(&merge_log);
    std::env::set_var("GWT_MOCK_MERGE_OID", SHA);
    let mut monitor = reviewed_monitor();

    // Tick 1: Reviewing → real fetchers (mock gh) → real gate → real merge_pr_auto.
    advance_autonomous_in_flight(&mut monitor, &issues, "test/repo", &repo, b"secret", now);
    assert_eq!(
        monitor.autonomous_record(42).map(|r| r.phase),
        Some(AutonomousPhase::Delivering),
        "gate passed against live (mock) gh ⇒ Delivering",
    );
    let log = fs::read_to_string(&merge_log).unwrap_or_default();
    assert!(
        log.contains("MERGE") && log.contains("pr merge"),
        "the real merge_pr_auto invoked `gh pr merge --auto` (log={log:?})",
    );

    // Tick 2: Delivering → real merge-commit fetch (mock gh) → merged==reviewed ⇒ done.
    advance_autonomous_in_flight(&mut monitor, &issues, "test/repo", &repo, b"secret", now);
    assert!(
        monitor.autonomous_record(42).is_none(),
        "merged_sha == reviewed_sha ⇒ record cleared (autonomous completion)",
    );
    assert_eq!(
        monitor.inbox_item(42).map(|i| i.state),
        Some(MonitorInboxState::Merged),
    );

    // --- Scenario B: merge lands on a DIFFERENT sha → layer-4 escalation ---
    let _ = fs::remove_file(&merge_log);
    std::env::set_var("GWT_MOCK_MERGE_OID", "deadbeef");
    let mut monitor = reviewed_monitor();
    advance_autonomous_in_flight(&mut monitor, &issues, "test/repo", &repo, b"secret", now);
    advance_autonomous_in_flight(&mut monitor, &issues, "test/repo", &repo, b"secret", now);
    assert_eq!(
        monitor.autonomous_record(42).map(|r| r.phase),
        Some(AutonomousPhase::NeedsHuman),
        "merged_sha != reviewed_sha ⇒ security escalation, NOT completion",
    );
    assert_eq!(
        monitor.inbox_item(42).map(|i| i.state),
        Some(MonitorInboxState::NeedsHuman),
    );

    cleanup(&tmp, &orig_path);
}

fn cleanup(tmp: &Path, orig_path: &str) {
    std::env::set_var("PATH", orig_path);
    std::env::remove_var("GWT_MOCK_GH_LOG");
    std::env::remove_var("GWT_MOCK_MERGE_OID");
    let _ = fs::remove_dir_all(tmp);
}
