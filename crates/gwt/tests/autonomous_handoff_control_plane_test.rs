//! Issue #3478 (SPEC #3200 FR-025): Issue Monitor control-plane half of the
//! question handoff — applying a hook-written handoff must park the owner
//! Issue for a human and free its active slot immediately, and answering it
//! must resume the exact same session instead of launching a duplicate.

use gwt::autonomous_handoff::{
    AutonomousExecutionContext, AutonomousHandoffState, AutonomousQuestionHandoff,
    ExtractedQuestion,
};
use gwt::{
    AutonomousPhase, EligibilityDecision, IssueMonitorConfig, IssueMonitorIssue,
    IssueMonitorIssueState, IssueMonitorPrefs, IssueMonitorState, MonitorInboxState,
};
use gwt_git::branch_protection::BranchProtectionStatus;

const BODY: &str = "## Acceptance Criteria\n- [ ] AC-1: the API returns 200\n";
const NOW: &str = "2026-08-06T05:00:00Z";

fn auto_issue(number: u64) -> IssueMonitorIssue {
    IssueMonitorIssue {
        number,
        title: format!("Issue {number}"),
        labels: vec!["auto-merge".to_string()],
        state: IssueMonitorIssueState::Open,
        body: Some(BODY.to_string()),
        url: None,
        readiness: gwt::IssueMonitorReadiness::NotApplicable,
    }
}

fn verified() -> BranchProtectionStatus {
    BranchProtectionStatus::Verified {
        required_checks: vec!["build".to_string()],
    }
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

fn handoff(issue_number: u64, session_id: &str) -> AutonomousQuestionHandoff {
    AutonomousQuestionHandoff::new(
        format!("handoff-{issue_number}"),
        &AutonomousExecutionContext {
            issue_number,
            session_id: session_id.to_string(),
        },
        "claude-code",
        "AskUserQuestion",
        ExtractedQuestion {
            question: "Should the release be published now?".to_string(),
            options: Vec::new(),
        },
        NOW,
    )
}

/// Drive one issue into an in-flight autonomous launch holding an active slot.
fn launch_autonomous(monitor: &mut IssueMonitorState, issue: &IssueMonitorIssue) {
    gwt::scan_issue_monitor_candidates(monitor, std::slice::from_ref(issue), NOW);
    let decision = monitor.prepare_autonomous_candidate(issue, &verified(), NOW);
    assert_eq!(decision, EligibilityDecision::Eligible);
    monitor.complete_active_launch(issue.number, "tab-1::agent-1");
    assert_eq!(monitor.active_count(), 1, "issue holds an active slot");
}

/// AC-4/AC-7: a pending handoff parks the Issue as NeedsHuman and releases the
/// slot in the same pass — no `stuck_timeout_secs` wait.
#[test]
fn pending_handoff_parks_the_issue_and_frees_the_slot_immediately() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);

    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    let parked = monitor.apply_pending_autonomous_handoffs(NOW);

    assert_eq!(parked, vec![42]);
    assert_eq!(
        monitor.autonomous_record(42).unwrap().phase,
        AutonomousPhase::NeedsHuman
    );
    assert_eq!(
        monitor.active_count(),
        0,
        "slot released for the next issue"
    );
    assert_eq!(
        monitor.inbox_item(42).map(|item| item.state),
        Some(MonitorInboxState::NeedsHuman)
    );

    let stored = monitor
        .open_autonomous_handoff(42)
        .expect("handoff stays addressable");
    assert_eq!(stored.state, AutonomousHandoffState::AwaitingHuman);
    assert_eq!(stored.session_id, "session-abc");

    // AC-7: the stuck-timeout fallback has nothing left to reclaim, and it is
    // reached long before `stuck_timeout_secs` (1800s) would have elapsed.
    assert!(monitor
        .stuck_autonomous_issues("2026-08-06T05:00:10Z")
        .is_empty());
}

/// Applying twice must not double-escalate or resurrect a freed slot.
#[test]
fn applying_pending_handoffs_is_idempotent() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);

    assert_eq!(monitor.apply_pending_autonomous_handoffs(NOW), vec![42]);
    assert!(monitor
        .apply_pending_autonomous_handoffs("2026-08-06T05:01:00Z")
        .is_empty());
    assert_eq!(monitor.active_count(), 0);
}

/// AC-6 non-regression: with autonomous mode OFF the control plane never
/// mutates lifecycle state, exactly like every other autonomous transition.
#[test]
fn handoffs_are_not_applied_when_autonomous_mode_is_off() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
    gwt::scan_issue_monitor_candidates(&mut monitor, &[auto_issue(42)], NOW);
    monitor.complete_active_launch(42, "tab-1::agent-1");

    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);

    assert!(monitor.apply_pending_autonomous_handoffs(NOW).is_empty());
    assert_eq!(monitor.active_count(), 1, "human-gated slot untouched");
    assert!(monitor.autonomous_record(42).is_none());
}

/// AC-5: registering an answer clears NeedsHuman and re-arms the SAME session
/// for a resume — the issue is queued once and carries the answer prompt.
#[test]
fn answering_a_handoff_resumes_the_same_session_without_a_duplicate_launch() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);

    assert!(monitor.answer_autonomous_handoff(
        "handoff-42",
        "Yes — publish it",
        "2026-08-06T06:00:00Z"
    ));

    let resumed = monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].issue_number, 42);
    assert_eq!(resumed[0].session_id, "session-abc");
    assert!(resumed[0]
        .prompt
        .contains("Should the release be published now?"));
    assert!(resumed[0].prompt.contains("Yes — publish it"));

    assert_eq!(
        monitor.autonomous_record(42).unwrap().phase,
        AutonomousPhase::Implementing,
        "the parked attempt is un-parked, not restarted"
    );
    assert!(
        monitor.open_autonomous_handoff(42).is_none(),
        "the handoff no longer blocks the issue"
    );
    assert_eq!(
        monitor
            .queued_issue_numbers()
            .iter()
            .filter(|n| **n == 42)
            .count(),
        1,
        "queued exactly once — no duplicate launch"
    );
}

/// AC-4/AC-8 (the point of the whole change): parking one Issue on a question
/// lets the Monitor launch the NEXT ready Issue on the same pass, instead of
/// the queue stalling until the stuck timeout expires.
#[test]
fn parking_a_question_lets_the_next_ready_issue_launch_immediately() {
    let mut monitor = autonomous_monitor();
    monitor.set_gui_connected(true);
    let blocked = auto_issue(42);
    let next_ready = auto_issue(43);
    gwt::scan_issue_monitor_candidates(&mut monitor, &[blocked.clone(), next_ready], NOW);
    monitor.prepare_autonomous_candidate(&blocked, &verified(), NOW);
    monitor.complete_active_launch(42, "tab-1::agent-1");

    // max_active is 1: while #42 holds the slot nothing else can start.
    assert_eq!(monitor.active_count(), 1);
    assert!(
        monitor.next_launch_request(NOW).is_none(),
        "the queue is stalled while the question-blocked issue holds the slot"
    );

    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);

    let next = monitor
        .next_launch_request(NOW)
        .expect("the freed slot admits the next ready issue");
    assert_eq!(next.issue_number, 43);
}

/// AC-5: the answer is delivered to the resumed launch exactly once, so a
/// second scan cannot replay it into the session.
#[test]
fn the_resume_prompt_is_delivered_to_the_launch_path_exactly_once() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes — publish it", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");

    let first = monitor.take_autonomous_resume_prompt(42, "2026-08-06T06:00:02Z");
    assert!(first
        .as_deref()
        .is_some_and(|prompt| prompt.contains("Yes — publish it")));
    assert!(
        monitor
            .take_autonomous_resume_prompt(42, "2026-08-06T06:00:03Z")
            .is_none(),
        "an already-delivered answer must not be replayed"
    );
}

/// The same one-shot delivery works across processes: the GUI launch path
/// reads and marks the prefs the daemon wrote.
#[test]
fn the_resume_prompt_is_taken_from_the_control_plane_file_exactly_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");

    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes — publish it", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist prefs");

    let first =
        gwt::take_autonomous_resume_prompt_from_prefs(&prefs_path, 42, "2026-08-06T06:00:02Z");
    assert!(first
        .as_deref()
        .is_some_and(|prompt| prompt.contains("Yes — publish it")));
    assert!(
        gwt::take_autonomous_resume_prompt_from_prefs(&prefs_path, 42, "2026-08-06T06:00:03Z")
            .is_none()
    );
}

/// Answering an unknown handoff id is rejected instead of silently succeeding.
#[test]
fn answering_an_unknown_handoff_is_rejected() {
    let mut monitor = autonomous_monitor();
    assert!(!monitor.answer_autonomous_handoff("nope", "answer", NOW));
    assert!(monitor.resume_answered_autonomous_handoffs(NOW).is_empty());
}

/// AC-8 (restart restore): handoffs survive the prefs round-trip with their
/// state, answer, and owning session intact.
#[test]
fn handoffs_survive_the_prefs_round_trip() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);

    let prefs = monitor.prefs();
    let encoded = serde_json::to_string(&prefs).expect("serialize prefs");
    let decoded: IssueMonitorPrefs = serde_json::from_str(&encoded).expect("deserialize prefs");
    assert_eq!(decoded.autonomous_handoffs.len(), 1);

    let restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), decoded);
    let restored_handoff = restored
        .open_autonomous_handoff(42)
        .expect("handoff restored after restart");
    assert_eq!(
        restored_handoff.state,
        AutonomousHandoffState::AwaitingHuman
    );
    assert_eq!(restored_handoff.session_id, "session-abc");
}

/// AC-4/AC-8 (restart): after a process restart the parked Issue must stay
/// parked. A rescan rebuilds the inbox from scratch, so the restored park has
/// to survive it — otherwise the restart would silently relaunch the work the
/// human was asked about.
#[test]
fn a_parked_issue_stays_parked_across_a_restart_and_rescan() {
    let mut monitor = autonomous_monitor();
    monitor.set_gui_connected(true);
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);

    // Restart: only the persisted prefs survive; the inbox is rebuilt by scan.
    let restarted_prefs = monitor.prefs();
    let mut restarted =
        IssueMonitorState::with_prefs(IssueMonitorConfig::default(), restarted_prefs);
    restarted.set_gui_connected(true);
    gwt::scan_issue_monitor_candidates(
        &mut restarted,
        std::slice::from_ref(&issue),
        "2026-08-06T07:00:00Z",
    );

    assert_eq!(
        restarted.inbox_item(42).map(|item| item.state),
        Some(MonitorInboxState::NeedsHuman),
        "the rescan must not revive a parked issue as Queued"
    );
    assert!(
        !restarted.queued_issue_numbers().contains(&42),
        "a parked issue must not be re-queued after a restart"
    );
    assert!(
        restarted.next_launch_request("2026-08-06T07:00:01Z").is_none(),
        "a parked issue must never be relaunched by the restarted driver"
    );
    assert!(
        restarted.open_autonomous_handoff(42).is_some(),
        "the question is still addressable after the restart"
    );
}

/// Older prefs written before this field exists must still deserialize.
#[test]
fn prefs_without_the_handoff_field_still_deserialize() {
    let prefs: IssueMonitorPrefs =
        serde_json::from_str(r#"{"enabled":true,"max_active_agents":1,"priority_order":[]}"#)
            .expect("legacy prefs deserialize");
    assert!(prefs.autonomous_handoffs.is_empty());
}

/// AC-9: the status view exposes the reason, the waiting question, the owning
/// session, and whether the work can be resumed — machine-readable and English.
#[test]
fn status_view_exposes_the_needs_human_question_and_resumability() {
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);

    let view = monitor.status_view();
    let summary = view
        .autonomous_issues
        .iter()
        .find(|summary| summary.issue_number == 42)
        .expect("issue summarized");

    assert!(summary.needs_human);
    let question = summary
        .pending_question
        .as_ref()
        .expect("waiting question exposed");
    assert_eq!(question.question, "Should the release be published now?");
    assert_eq!(question.session_id, "session-abc");
    assert_eq!(question.reason_code, "external_side_effect");
    assert!(question.resumable, "answering resumes the stored session");
    assert!(summary
        .needs_human_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("human judgment")));
}
