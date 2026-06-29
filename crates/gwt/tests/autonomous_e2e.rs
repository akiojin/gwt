//! SPEC #3200 T-100 — end-to-end verification of the autonomous Issue Monitor
//! decision loop. This drives a complete autonomous resolution through every
//! phase (eligibility → Implementing → Reviewing → gate → Delivering → merge)
//! and asserts the core SAFETY property: the loop reaches a merge ONLY when the
//! full strong gate passes, and escalates / retries / waits on every failure.
//!
//! The real `gh` subprocess and agent spawn are thin adapters (separately
//! injectable-tested); this exercises the dangerous part — the decision pipeline
//! — deterministically, with no network.

use gwt::issue_monitor_authz::merged_sha_matches_reviewed;
use gwt::issue_monitor_gate::{
    evaluate_autonomous_gate, route_autonomous_gate, GateAction, GateDecision,
};
use gwt::{
    AutonomousPhase, EligibilityDecision, IssueMonitorConfig, IssueMonitorIssue,
    IssueMonitorIssueState, IssueMonitorPrefs, IssueMonitorState, MonitorInboxState,
};
use gwt_git::branch_protection::BranchProtectionStatus;

const BODY: &str = "## Acceptance Criteria\n- [ ] AC-1: the API returns 200\n";
const SHA: &str = "abc123";

fn auto_issue(number: u64) -> IssueMonitorIssue {
    IssueMonitorIssue {
        number,
        title: format!("Issue {number}"),
        labels: vec!["auto-merge".to_string()],
        state: IssueMonitorIssueState::Open,
        body: Some(BODY.to_string()),
        url: None,
    }
}

fn verified() -> BranchProtectionStatus {
    BranchProtectionStatus::Verified {
        required_checks: vec!["build".to_string()],
    }
}

fn pass_rollup() -> &'static str {
    r#"[{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}]"#
}

fn autonomous_monitor() -> IssueMonitorState {
    IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        IssueMonitorPrefs {
            autonomous_mode: true,
            ..IssueMonitorPrefs::default()
        },
    )
}

/// Drive an issue from eligibility through Implementing + Reviewing with a
/// passing verdict, leaving it ready for the gate.
fn drive_to_reviewed(
    monitor: &mut IssueMonitorState,
    issue: &IssueMonitorIssue,
    review_passes: bool,
) {
    // Scan the issue into the inbox first so inbox-state transitions are visible.
    gwt::scan_issue_monitor_candidates(
        monitor,
        std::slice::from_ref(issue),
        "2026-06-29T00:00:00Z",
    );
    let decision = monitor.prepare_autonomous_candidate(issue, &verified(), "2026-06-29T00:00:00Z");
    assert_eq!(decision, EligibilityDecision::Eligible, "eligible to start");
    assert_eq!(
        monitor.autonomous_record(issue.number).unwrap().phase,
        AutonomousPhase::Implementing,
    );
    // Implementation produced a PR at SHA.
    monitor.begin_review(issue.number, 99, SHA);
    assert_eq!(
        monitor.autonomous_record(issue.number).unwrap().phase,
        AutonomousPhase::Reviewing,
    );
    // Independent review reports a verdict for that SHA (daemon-judged).
    monitor.record_review_verdict(issue.number, review_passes);
}

#[test]
fn full_pass_reaches_deliver_and_completes_only_on_sha_match() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    drive_to_reviewed(&mut monitor, &issue, true);

    // Gate assembly + routing: all conditions hold ⇒ Deliver.
    let inputs = monitor
        .autonomous_gate_inputs(42, verified(), pass_rollup(), SHA, BODY)
        .expect("gate ready once verdict is in");
    assert_eq!(evaluate_autonomous_gate(&inputs), GateDecision::Pass);
    assert_eq!(route_autonomous_gate(&inputs), GateAction::Deliver);

    monitor.begin_delivering(42);
    assert_eq!(
        monitor.autonomous_record(42).unwrap().phase,
        AutonomousPhase::Delivering,
    );

    // Merge watch: merged SHA must equal the reviewed SHA before completing.
    assert!(
        merged_sha_matches_reviewed(SHA, SHA),
        "merged==reviewed ⇒ may complete",
    );
    monitor.record_merged(42);
    assert_eq!(
        monitor.inbox_item(42).map(|i| i.state),
        Some(MonitorInboxState::Merged),
    );
    assert!(
        monitor.autonomous_record(42).is_none(),
        "completion clears the record",
    );
    assert!(monitor.autonomous_in_flight_issues().is_empty());
}

#[test]
fn merge_sha_drift_blocks_completion() {
    // SPEC #3200 layer-4: if the SHA that actually merged differs from the
    // reviewed SHA, the loop must NOT treat it as a clean completion.
    assert!(!merged_sha_matches_reviewed(SHA, "def456"));
}

#[test]
fn rejected_review_does_not_reach_deliver() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    drive_to_reviewed(&mut monitor, &issue, false); // review FAILED

    let inputs = monitor
        .autonomous_gate_inputs(42, verified(), pass_rollup(), SHA, BODY)
        .expect("gate ready");
    assert!(matches!(
        evaluate_autonomous_gate(&inputs),
        GateDecision::Fail(_)
    ));
    assert!(matches!(
        route_autonomous_gate(&inputs),
        GateAction::Remediate(_)
    ));
}

#[test]
fn acceptance_drift_after_launch_does_not_reach_deliver() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    drive_to_reviewed(&mut monitor, &issue, true);

    // The Issue body was edited after launch (criteria changed).
    let drifted_body = "## Acceptance Criteria\n- [ ] AC-2: a different requirement\n";
    let inputs = monitor
        .autonomous_gate_inputs(42, verified(), pass_rollup(), SHA, drifted_body)
        .expect("gate ready");
    assert!(matches!(
        evaluate_autonomous_gate(&inputs),
        GateDecision::Fail(_)
    ));
    assert!(matches!(
        route_autonomous_gate(&inputs),
        GateAction::Escalate(_)
    ));
}

#[test]
fn head_advance_after_review_does_not_reach_deliver() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    drive_to_reviewed(&mut monitor, &issue, true);

    // HEAD advanced: the current head SHA no longer equals the reviewed SHA.
    let inputs = monitor
        .autonomous_gate_inputs(42, verified(), pass_rollup(), "def456", BODY)
        .expect("gate ready");
    assert!(matches!(
        evaluate_autonomous_gate(&inputs),
        GateDecision::Fail(_)
    ));
    // A HEAD advance is agent-remediable (re-review the new SHA).
    assert!(matches!(
        route_autonomous_gate(&inputs),
        GateAction::Remediate(_)
    ));
}

#[test]
fn pending_ci_waits_does_not_merge() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    drive_to_reviewed(&mut monitor, &issue, true);

    let pending_rollup = r#"[{"name":"build","status":"IN_PROGRESS","conclusion":null}]"#;
    let inputs = monitor
        .autonomous_gate_inputs(42, verified(), pending_rollup, SHA, BODY)
        .expect("gate ready");
    assert!(matches!(
        evaluate_autonomous_gate(&inputs),
        GateDecision::Fail(_)
    ));
    assert_eq!(route_autonomous_gate(&inputs), GateAction::WaitForCi);
}

#[test]
fn unverified_branch_protection_blocks_eligibility() {
    // The whole loop never even starts if branch protection is not verified.
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    let decision = monitor.prepare_autonomous_candidate(
        &issue,
        &BranchProtectionStatus::Absent,
        "2026-06-29T00:00:00Z",
    );
    assert!(matches!(decision, EligibilityDecision::NeedsHuman(_)));
    assert_eq!(
        monitor.autonomous_record(42).map(|r| r.phase),
        Some(AutonomousPhase::NeedsHuman),
    );
}

#[test]
fn gate_not_ready_until_verdict_returns() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    let decision =
        monitor.prepare_autonomous_candidate(&issue, &verified(), "2026-06-29T00:00:00Z");
    assert_eq!(decision, EligibilityDecision::Eligible);
    monitor.begin_review(42, 99, SHA); // verdict pending
    assert!(
        monitor
            .autonomous_gate_inputs(42, verified(), pass_rollup(), SHA, BODY)
            .is_none(),
        "no gate decision while the independent review is still in flight",
    );
}

#[test]
fn default_off_never_enters_the_autonomous_loop() {
    // SPEC #3165 non-regression: with autonomous_mode OFF, an auto-merge-labelled
    // issue is NOT an autonomous candidate and no autonomous state is created.
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
    let issue = auto_issue(42);
    let decision =
        monitor.prepare_autonomous_candidate(&issue, &verified(), "2026-06-29T00:00:00Z");
    assert!(matches!(decision, EligibilityDecision::HumanGate(_)));
    assert!(monitor.autonomous_record(42).is_none());
    assert!(monitor.autonomous_in_flight_issues().is_empty());
}
