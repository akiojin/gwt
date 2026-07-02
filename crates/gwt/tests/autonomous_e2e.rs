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

#[test]
fn fr_family_decision_boundaries_are_observable() {
    // SPEC #3200 T-104 / FR-033 testability audit: every autonomous decision
    // boundary must be observable from the outside — via a state transition, an
    // inbox surface, a returned verification value, or an operator notice. One
    // assertion block per FR family; if a future refactor makes a boundary
    // silent, this audit goes RED.
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);

    // FR-001/FR-003 (two-stage opt-in): the eligibility DECISION is a returned
    // value, and a non-candidate leaves no autonomous state behind.
    let mut plain = auto_issue(43);
    plain.labels.clear(); // no auto-merge label ⇒ human gate
    let ineligible =
        monitor.prepare_autonomous_candidate(&plain, &verified(), "2026-07-02T00:00:00Z");
    assert!(matches!(ineligible, EligibilityDecision::HumanGate(_)));
    assert!(monitor.autonomous_record(43).is_none());

    // FR-006/FR-014 (eligibility + acceptance snapshot): eligibility is a
    // returned value; the captured snapshot + Implementing phase are state.
    gwt::scan_issue_monitor_candidates(
        &mut monitor,
        std::slice::from_ref(&issue),
        "2026-07-02T00:00:00Z",
    );
    let eligible =
        monitor.prepare_autonomous_candidate(&issue, &verified(), "2026-07-02T00:00:00Z");
    assert_eq!(eligible, EligibilityDecision::Eligible);
    let record = monitor.autonomous_record(42).expect("record created");
    assert_eq!(record.phase, AutonomousPhase::Implementing);
    assert!(record.acceptance_snapshot.is_some(), "snapshot observable");

    // FR-026 (attempt counter) + FR-022/FR-024/FR-029 (transient retry/backoff):
    // the failure outcome is a returned value; the counter, the backoff window,
    // and a warn notice are all observable.
    let outcome = monitor.record_autonomous_failure(
        42,
        gwt::FailureClass::Transient,
        "transient blip",
        "2026-07-02T00:10:00Z",
    );
    assert!(matches!(
        outcome,
        gwt::issue_monitor::AutonomousFailureOutcome::Retry { attempt: 1 }
    ));
    assert_eq!(monitor.attempt_count(42), 1);
    assert!(!monitor.retry_ready(42, "2026-07-02T00:10:01Z"));

    // FR-013/FR-025 (stuck/idle detection): the stuck set is a queryable value
    // anchored on the heartbeat.
    monitor.prepare_autonomous_candidate(&issue, &verified(), "2026-07-02T02:00:00Z");
    monitor.complete_active_launch(42, "tab-1::agent-42");
    monitor.record_autonomous_heartbeat(42, "2026-07-02T02:00:00Z");
    assert!(
        monitor
            .stuck_autonomous_issues("2026-07-02T09:00:00Z")
            .contains(&42),
        "stale heartbeat is observable as stuck"
    );

    // FR-009..FR-016 (strong gate): the gate decision and route are returned
    // values bound to the reviewed SHA.
    monitor.begin_review(42, 99, SHA);
    monitor.record_review_verdict(42, true);
    let inputs = monitor
        .autonomous_gate_inputs(42, verified(), pass_rollup(), SHA, BODY)
        .expect("gate ready");
    assert_eq!(evaluate_autonomous_gate(&inputs), GateDecision::Pass);
    assert_eq!(route_autonomous_gate(&inputs), GateAction::Deliver);

    // FR-032/FR-033 (status protocol): mode + per-issue phase/attempts are on
    // the status view.
    let status = monitor.status_view();
    assert!(status.autonomous_mode);
    let summary = status
        .autonomous_issues
        .iter()
        .find(|entry| entry.issue_number == 42)
        .expect("per-issue autonomous summary observable");
    assert_eq!(summary.attempts, 1);

    // FR-017/FR-018 (delivery + layer-4) + FR-034 (notices): arming, the
    // SHA-identity check, and completion are observable; the operator notices
    // for retry / arming / completion are queued for the GUI.
    monitor.begin_delivering(42);
    monitor.record_auto_merge_armed(42); // arm succeeded ⇒ info notice
    assert_eq!(
        monitor.autonomous_record(42).unwrap().phase,
        AutonomousPhase::Delivering
    );
    assert!(merged_sha_matches_reviewed(SHA, SHA));
    monitor.record_merged(42);
    assert_eq!(
        monitor.inbox_item(42).map(|i| i.state),
        Some(MonitorInboxState::Merged)
    );
    let notice_levels: Vec<String> = monitor
        .take_autonomous_notices()
        .into_iter()
        .map(|notice| notice.level)
        .collect();
    for expected in ["warn", "info", "done"] {
        assert!(
            notice_levels.iter().any(|level| level == expected),
            "operator notice `{expected}` observable: {notice_levels:?}"
        );
    }

    // FR-027 (NeedsHuman escalation): terminal failure is observable via the
    // inbox state, the status view, and an error notice.
    let mut escalated = autonomous_monitor();
    let issue2 = auto_issue(44);
    gwt::scan_issue_monitor_candidates(
        &mut escalated,
        std::slice::from_ref(&issue2),
        "2026-07-02T00:00:00Z",
    );
    escalated.prepare_autonomous_candidate(&issue2, &verified(), "2026-07-02T00:00:00Z");
    let terminal = escalated.record_autonomous_failure(
        44,
        gwt::FailureClass::Terminal,
        "review rejected",
        "2026-07-02T00:30:00Z",
    );
    assert!(matches!(
        terminal,
        gwt::issue_monitor::AutonomousFailureOutcome::Escalated(_)
    ));
    assert_eq!(
        escalated.inbox_item(44).map(|i| i.state),
        Some(MonitorInboxState::NeedsHuman)
    );
    let status = escalated.status_view();
    assert!(status
        .autonomous_issues
        .iter()
        .any(|entry| entry.issue_number == 44 && entry.needs_human));
    assert!(escalated
        .take_autonomous_notices()
        .iter()
        .any(|notice| notice.level == "error" && notice.issue_number == 44));

    // FR-002 (kill switch observability): flipping the mode off is visible on
    // the status view.
    escalated.set_autonomous_mode(false);
    assert!(!escalated.status_view().autonomous_mode);
}
