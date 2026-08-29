//! Issue #3478 (SPEC #3200 FR-025): Issue Monitor control-plane half of the
//! question handoff — applying a hook-written handoff must park the owner
//! Issue for a human and free its active slot immediately, and answering it
//! must resume the exact same session instead of launching a duplicate.

use gwt::autonomous_handoff::{
    parse_protected_autonomous_handoff_answer_prompt, AutonomousExecutionContext,
    AutonomousHandoffDeliveryState, AutonomousHandoffDeliveryTarget,
    AutonomousHandoffReceiptIdentity, AutonomousHandoffState, AutonomousQuestionHandoff,
    ExtractedQuestion,
};
use gwt::{
    AutonomousHandoffDeliveryFailureOutcome, AutonomousHandoffDeliveryPreparation, AutonomousPhase,
    EligibilityDecision, IssueMonitorConfig, IssueMonitorIssue, IssueMonitorIssueState,
    IssueMonitorPrefs, IssueMonitorState, MonitorInboxState,
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
        updated_at: None,
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

fn answer_delivery_identities(
    issue_number: u64,
) -> (
    AutonomousHandoffDeliveryTarget,
    AutonomousHandoffReceiptIdentity,
) {
    let target = AutonomousHandoffDeliveryTarget {
        gwt_session_id: "resumed-session".to_string(),
        native_session_id: "native-session".to_string(),
        provider: "claude-code".to_string(),
        issue_number,
        repo_hash: "repo-hash".to_string(),
        project_state_root: "/project-state".to_string(),
        window_id: "tab-1::agent-answer".to_string(),
        materializer_id: "test-materializer".to_string(),
        materializer_pid: std::process::id(),
        materializer_started_at: gwt::process::host_process_start_time(std::process::id())
            .expect("test materializer start time"),
        delivery_id: None,
    };
    let receipt = AutonomousHandoffReceiptIdentity {
        gwt_session_id: target.gwt_session_id.clone(),
        native_session_id: target.native_session_id.clone(),
        provider: target.provider.clone(),
        issue_number,
        repo_hash: target.repo_hash.clone(),
        project_state_root: target.project_state_root.clone(),
    };
    (target, receipt)
}

fn bind_answer_delivery(
    prefs_path: &std::path::Path,
    prepared: &gwt::AutonomousHandoffDeliveryAttempt,
) -> AutonomousHandoffReceiptIdentity {
    let (target, receipt) = answer_delivery_identities(prepared.issue_number);
    assert!(gwt::bind_autonomous_handoff_delivery_target_from_prefs(
        prefs_path,
        &prepared.handoff_id,
        &prepared.session_id,
        prepared.attempt,
        &target,
    )
    .expect("bind answer target"));
    receipt
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
    // SPEC-3431 FR-068 starts the activity clock for every launch, so a bare
    // record (heartbeat only) may exist; what must not happen is the handoff
    // application itself — no park, no NeedsHuman phase.
    assert!(monitor
        .autonomous_record(42)
        .is_none_or(|record| record.phase != AutonomousPhase::NeedsHuman));
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
    monitor.set_gui_connected(true);
    assert_eq!(
        monitor
            .next_launch_request("2026-08-06T06:00:02Z")
            .expect("answered handoff launch request")
            .launch_session_strategy,
        gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe,
        "a human answer must override any stale fresh-session retry policy",
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
    monitor.complete_active_launch(42, "tab-1::agent-answer");

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

/// Issue #3716 AC-2: delivery is write-ahead durable before submit and only an
/// authenticated UserPromptSubmit receipt for the exact asking Session can
/// commit it.
#[test]
fn autonomous_answer_delivery_prepares_and_receipts_the_exact_handoff() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes — publish it", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    monitor.complete_active_launch(42, "tab-1::agent-answer");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist prefs");

    let prepared = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:02Z",
    )
    .expect("prepare delivery")
    .expect("pending exact handoff")
    {
        AutonomousHandoffDeliveryPreparation::Ready(prepared) => prepared,
        other => panic!("expected Ready, got {other:?}"),
    };
    let receipt_identity = bind_answer_delivery(&prefs_path, &prepared);
    assert_eq!(prepared.handoff_id, "handoff-42");
    assert_eq!(prepared.session_id, "session-abc");
    assert_eq!(prepared.attempt, 1);
    assert!(prepared.prompt.contains("Yes — publish it"));
    let marker = parse_protected_autonomous_handoff_answer_prompt(&prepared.prompt)
        .expect("protected answer marker");
    assert_eq!(marker.handoff_id, "handoff-42");
    assert_eq!(marker.session_id, "session-abc");
    assert_eq!(marker.attempt, 1);
    assert!(parse_protected_autonomous_handoff_answer_prompt(
        &prepared
            .prompt
            .replace("Yes — publish it", "No — forged answer")
    )
    .is_none());
    let after_prepare = gwt::load_issue_monitor_prefs(&prefs_path).expect("load prepared delivery");
    assert!(after_prepare.autonomous_handoffs[0].delivered_at.is_none());
    assert!(matches!(
        after_prepare.autonomous_handoffs[0].delivery,
        AutonomousHandoffDeliveryState::Attempting { attempt: 1, .. }
    ));

    assert!(
        !gwt::acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
            &prefs_path,
            "different-session",
            &receipt_identity,
            &prepared.prompt,
            "2026-08-06T06:00:03Z",
        )
        .expect("reject mismatched Session")
    );
    let mut forged_target = receipt_identity.clone();
    forged_target.project_state_root = "/foreign-project".to_string();
    assert!(
        !gwt::acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
            &prefs_path,
            "session-abc",
            &forged_target,
            &prepared.prompt,
            "2026-08-06T06:00:03Z",
        )
        .expect("reject mismatched project identity")
    );
    assert!(
        gwt::acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
            &prefs_path,
            "session-abc",
            &receipt_identity,
            &prepared.prompt,
            "2026-08-06T06:00:04Z",
        )
        .expect("acknowledge exact UserPromptSubmit")
    );
    let delivered = gwt::load_issue_monitor_prefs(&prefs_path).expect("load delivered handoff");
    assert_eq!(
        delivered.autonomous_handoffs[0].delivered_at.as_deref(),
        Some("2026-08-06T06:00:04Z")
    );
    assert!(matches!(
        delivered.autonomous_handoffs[0].delivery,
        AutonomousHandoffDeliveryState::Delivered { attempt: 1, .. }
    ));
}

/// A semantic receipt may beat the physical materialization callback. It
/// commits the answer but leaves the pending delivery intact until the window
/// and workspace are both durable.
#[test]
fn early_receipt_does_not_complete_an_undurable_launch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    assert!(monitor.apply_confirmed_claim(
        42,
        "claim-42",
        "host/session",
        "answer-effect-42",
        "2026-08-06T06:00:02Z",
    ));
    let delivery_id = monitor
        .pending_launch_delivery_id(42)
        .expect("pending launch delivery");
    assert!(monitor.claim_launch_delivery(
        42,
        &delivery_id,
        "test-materializer",
        std::process::id(),
        "tab-1::agent-answer",
        gwt::process::is_host_process_alive,
    ));
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist launch");
    let prepared = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:03Z",
    )
    .expect("prepare")
    .expect("ready")
    {
        AutonomousHandoffDeliveryPreparation::Ready(prepared) => prepared,
        other => panic!("expected Ready, got {other:?}"),
    };
    let (mut target, receipt) = answer_delivery_identities(42);
    target.delivery_id = Some(delivery_id.clone());
    assert!(gwt::bind_autonomous_handoff_delivery_target_from_prefs(
        &prefs_path,
        &prepared.handoff_id,
        &prepared.session_id,
        prepared.attempt,
        &target,
    )
    .expect("bind exact pending target"));

    assert!(
        gwt::acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
            &prefs_path,
            "session-abc",
            &receipt,
            &prepared.prompt,
            "2026-08-06T06:00:04Z",
        )
        .expect("accept early receipt")
    );
    let receipt_prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load receipt state");
    assert!(receipt_prefs
        .pending_launch_deliveries
        .iter()
        .any(|delivery| delivery.delivery_id == delivery_id));
    assert!(receipt_prefs
        .launched_issues
        .iter()
        .all(|launched| launched.issue_number != 42));
    assert!(matches!(
        receipt_prefs.autonomous_handoffs[0].delivery,
        AutonomousHandoffDeliveryState::Delivered { .. }
    ));

    let mut materializer =
        IssueMonitorState::with_prefs(IssueMonitorConfig::default(), receipt_prefs);
    assert!(materializer.mark_launch_delivery_materialized(
        42,
        &delivery_id,
        "test-materializer",
        "tab-1::agent-answer",
    ));
    assert!(materializer.mark_launch_delivery_workspace_durable(
        42,
        &delivery_id,
        "test-materializer",
        "tab-1::agent-answer",
    ));
    assert!(materializer.complete_active_launch_delivery(
        42,
        "tab-1::agent-answer",
        Some(&delivery_id),
    ));
    assert_eq!(
        materializer.launched_window_issue("tab-1::agent-answer"),
        Some(42)
    );
}

/// Stop/disable revokes launch ownership. A delayed provider receipt may mark
/// nothing and must never recreate the released slot or Launched row.
#[test]
fn delayed_receipt_after_monitor_disable_cannot_resurrect_the_launch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    monitor.complete_active_launch(42, "tab-1::agent-answer");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist launch");
    let prepared = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:02Z",
    )
    .expect("prepare")
    .expect("ready")
    {
        AutonomousHandoffDeliveryPreparation::Ready(prepared) => prepared,
        other => panic!("expected Ready, got {other:?}"),
    };
    let receipt = bind_answer_delivery(&prefs_path, &prepared);
    let mut enabled_prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load bound attempt");
    enabled_prefs.enabled = true;
    let mut disabled = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), enabled_prefs);
    assert!(disabled.set_enabled_with_effect_revocation(false).is_some());
    gwt::save_issue_monitor_prefs(&prefs_path, &disabled.prefs()).expect("persist disabled state");

    assert!(
        !gwt::acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
            &prefs_path,
            "session-abc",
            &receipt,
            &prepared.prompt,
            "2026-08-06T06:00:03Z",
        )
        .expect("reject delayed receipt")
    );
    let after = gwt::load_issue_monitor_prefs(&prefs_path).expect("load stopped state");
    assert!(after
        .launched_issues
        .iter()
        .all(|launched| launched.issue_number != 42));
    assert!(after.autonomous_handoffs[0].delivered_at.is_none());
}

/// A resident daemon may still hold the pre-delivery snapshot when the GUI
/// commits the write-ahead fence. Rebasing that daemon must preserve the disk
/// fence so its next save cannot make the same answer replayable.
#[test]
fn stale_daemon_rebase_preserves_the_disk_delivery_fence_and_receipt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    monitor.complete_active_launch(42, "tab-1::agent-answer");
    let mut stale_daemon =
        IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist pending answer");

    let prepared = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:02Z",
    )
    .expect("prepare delivery")
    .expect("ready delivery")
    {
        AutonomousHandoffDeliveryPreparation::Ready(prepared) => prepared,
        other => panic!("expected Ready, got {other:?}"),
    };
    let receipt_identity = bind_answer_delivery(&prefs_path, &prepared);
    let disk = gwt::load_issue_monitor_prefs(&prefs_path).expect("load disk fence");
    stale_daemon.rebase_daemon_driver_prefs(&disk);
    assert!(matches!(
        stale_daemon.autonomous_handoffs()[0].delivery,
        AutonomousHandoffDeliveryState::Attempting { attempt: 1, .. }
    ));

    gwt::save_issue_monitor_prefs(&prefs_path, &stale_daemon.prefs())
        .expect("persist rebased daemon");
    assert!(
        gwt::acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
            &prefs_path,
            "session-abc",
            &receipt_identity,
            &prepared.prompt,
            "2026-08-06T06:00:03Z",
        )
        .expect("receipt remains valid after daemon save")
    );
}

/// Binding the exact target enriches the same Attempting attempt. A daemon
/// that already observed the unbound fence must not erase that enrichment.
#[test]
fn stale_daemon_rebase_preserves_same_attempt_target_binding() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    monitor.complete_active_launch(42, "tab-1::agent-answer");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist answer");
    let prepared = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:02Z",
    )
    .expect("prepare")
    .expect("ready")
    {
        AutonomousHandoffDeliveryPreparation::Ready(prepared) => prepared,
        other => panic!("expected Ready, got {other:?}"),
    };
    let unbound = gwt::load_issue_monitor_prefs(&prefs_path).expect("load unbound fence");
    bind_answer_delivery(&prefs_path, &prepared);
    let bound = gwt::load_issue_monitor_prefs(&prefs_path).expect("load bound fence");
    let mut stale_daemon = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), unbound);

    stale_daemon.rebase_daemon_driver_prefs(&bound);

    assert!(matches!(
        &stale_daemon.autonomous_handoffs()[0].delivery,
        AutonomousHandoffDeliveryState::Attempting {
            target: Some(_),
            ..
        }
    ));
}

/// The prepare fence itself carries a host incarnation, so a daemon scan in
/// the normal prepare-to-bind gap waits instead of manufacturing ambiguity.
#[test]
fn live_prebind_materializer_survives_a_concurrent_daemon_scan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist answer");
    assert!(matches!(
        gwt::prepare_autonomous_handoff_delivery_from_prefs(
            &prefs_path,
            42,
            "2026-08-06T06:00:02Z"
        )
        .expect("prepare")
        .expect("ready"),
        AutonomousHandoffDeliveryPreparation::Ready(_)
    ));
    let mut daemon = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        gwt::load_issue_monitor_prefs(&prefs_path).expect("load prebind fence"),
    );

    assert!(daemon
        .apply_pending_autonomous_handoffs("2026-08-06T06:00:03Z")
        .is_empty());
    assert!(matches!(
        daemon.autonomous_handoffs()[0].delivery,
        AutonomousHandoffDeliveryState::Attempting { target: None, .. }
    ));
}

/// A provider-authenticated receipt is stronger than restart-time ambiguity
/// inferred by a stale daemon for the same answer and attempt.
#[test]
fn delivered_receipt_dominates_stale_ambiguous_rebase() {
    let mut delivered = handoff(42, "session-abc");
    delivered.answer = Some("Yes".to_string());
    delivered.answered_at = Some("2026-08-06T06:00:00Z".to_string());
    delivered.answer_revision = 1;
    delivered.state = AutonomousHandoffState::Resumed;
    delivered.delivered_at = Some("2026-08-06T06:00:03Z".to_string());
    delivered.delivery = AutonomousHandoffDeliveryState::Delivered {
        attempt: 1,
        prompt_sha256: "a".repeat(64),
        delivered_at: "2026-08-06T06:00:03Z".to_string(),
    };
    let mut ambiguous = delivered.clone();
    ambiguous.delivered_at = None;
    ambiguous.state = AutonomousHandoffState::AwaitingHuman;
    ambiguous.delivery = AutonomousHandoffDeliveryState::Ambiguous {
        attempt: 1,
        prompt_sha256: "a".repeat(64),
        detected_at: "2026-08-06T06:00:02Z".to_string(),
        reason: "stale restart observer".to_string(),
    };
    let mut daemon = autonomous_monitor();
    daemon.absorb_autonomous_handoffs(vec![ambiguous]);

    daemon.absorb_autonomous_handoffs(vec![delivered]);

    assert!(matches!(
        daemon.autonomous_handoffs()[0].delivery,
        AutonomousHandoffDeliveryState::Delivered { .. }
    ));
    assert_eq!(
        daemon.autonomous_handoffs()[0].state,
        AutonomousHandoffState::Resumed
    );
}

/// Answer timestamps are display data. A monotonic revision preserves a
/// replacement even when both answers were recorded in the same second.
#[test]
fn same_second_replacement_answer_dominates_a_stale_daemon_copy() {
    let mut old = handoff(42, "session-abc");
    old.answer = Some("Old answer".to_string());
    old.answered_at = Some("2026-08-06T06:00:00Z".to_string());
    old.answer_revision = 1;
    old.state = AutonomousHandoffState::Answered;
    let mut replacement = old.clone();
    replacement.answer = Some("Replacement answer".to_string());
    replacement.answer_revision = 2;
    let mut daemon = autonomous_monitor();
    daemon.absorb_autonomous_handoffs(vec![old]);

    daemon.absorb_autonomous_handoffs(vec![replacement]);

    assert_eq!(
        daemon.autonomous_handoffs()[0].answer.as_deref(),
        Some("Replacement answer")
    );
    assert_eq!(daemon.autonomous_handoffs()[0].answer_revision, 2);
}

/// A newer answer may already have reached a terminal delivery outcome before
/// the resident daemon observes the revision. Its lifecycle companions must
/// be projected together with the record, not lost behind the revision jump.
#[test]
fn newer_answer_terminal_delivery_projects_needs_human_and_releases_the_slot() {
    let mut daemon = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut daemon, &issue);
    let mut old = handoff(42, "session-abc");
    old.answer = Some("Old answer".to_string());
    old.answered_at = Some("2026-08-06T06:00:00Z".to_string());
    old.answer_revision = 1;
    old.state = AutonomousHandoffState::Resumed;
    daemon.absorb_autonomous_handoffs(vec![old.clone()]);
    let mut failed_replacement = old;
    failed_replacement.answer = Some("Replacement answer".to_string());
    failed_replacement.answer_revision = 2;
    failed_replacement.state = AutonomousHandoffState::AwaitingHuman;
    failed_replacement.delivery = AutonomousHandoffDeliveryState::Ambiguous {
        attempt: 1,
        prompt_sha256: "b".repeat(64),
        detected_at: "2026-08-06T06:00:01Z".to_string(),
        reason: "provider exited before semantic receipt".to_string(),
    };

    daemon.absorb_autonomous_handoffs(vec![failed_replacement]);

    assert_eq!(daemon.active_count(), 0);
    assert_eq!(
        daemon.autonomous_record(42).map(|record| record.phase),
        Some(AutonomousPhase::NeedsHuman)
    );
    assert_eq!(
        daemon.inbox_item(42).map(|item| item.state),
        Some(MonitorInboxState::NeedsHuman)
    );
}

/// A crash after write-ahead preparation leaves the physical submit outcome
/// unknowable. Restart must never replay that answer automatically.
#[test]
fn unresolved_answer_attempt_becomes_ambiguous_and_frees_the_slot_on_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    // Recreate an in-flight launch holding the active slot when the process
    // crashes after the durable prepare but before a receipt.
    monitor.complete_active_launch(42, "tab-1::agent-answer");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist prefs");

    assert!(matches!(
        gwt::prepare_autonomous_handoff_delivery_from_prefs(
            &prefs_path,
            42,
            "2026-08-06T06:00:02Z"
        )
        .expect("prepare")
        .expect("ready"),
        AutonomousHandoffDeliveryPreparation::Ready(_)
    ));
    let mut prepared = gwt::load_issue_monitor_prefs(&prefs_path).expect("load prepared prefs");
    if let AutonomousHandoffDeliveryState::Attempting {
        materializer_pid,
        materializer_started_at,
        ..
    } = &mut prepared.autonomous_handoffs[0].delivery
    {
        *materializer_pid = 2_000_000_000;
        *materializer_started_at = 1;
    }
    let mut restarted = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prepared);
    assert_eq!(
        restarted.apply_pending_autonomous_handoffs("2026-08-06T06:10:00Z"),
        vec![42],
        "the daemon scan must reconcile an unresolved submit fence"
    );
    gwt::save_issue_monitor_prefs(&prefs_path, &restarted.prefs())
        .expect("persist restart reconciliation");

    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load reconciled prefs");
    assert!(prefs
        .launched_issues
        .iter()
        .all(|item| item.issue_number != 42));
    assert!(prefs
        .pending_launch_deliveries
        .iter()
        .all(|delivery| delivery.issue_number != 42));
    assert!(prefs
        .failed_issues
        .iter()
        .any(|failed| failed.issue_number == 42));
    assert!(prefs.autonomous_records.iter().any(|record| {
        record.issue_number == 42 && record.phase == AutonomousPhase::NeedsHuman
    }));
    assert!(matches!(
        prefs.autonomous_handoffs[0].delivery,
        AutonomousHandoffDeliveryState::Ambiguous { attempt: 1, .. }
    ));
    assert_eq!(
        prefs.autonomous_handoffs[0].state,
        AutonomousHandoffState::AwaitingHuman
    );
}

/// The daemon scans while the GUI is still submitting an answer. A live
/// materializer PID distinguishes that normal overlap from crash recovery.
#[test]
fn live_materializer_attempt_is_not_reconciled_as_a_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    monitor.complete_active_launch(42, "tab-1::agent-answer");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist answer");
    let prepared = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:02Z",
    )
    .expect("prepare")
    .expect("ready")
    {
        AutonomousHandoffDeliveryPreparation::Ready(prepared) => prepared,
        other => panic!("expected Ready, got {other:?}"),
    };
    bind_answer_delivery(&prefs_path, &prepared);
    assert!(matches!(
        gwt::prepare_autonomous_handoff_delivery_from_prefs(
            &prefs_path,
            42,
            "2026-08-06T06:00:03Z"
        )
        .expect("inspect live materializer")
        .expect("in-flight result"),
        AutonomousHandoffDeliveryPreparation::InFlight { attempt: 1, .. }
    ));
    let mut daemon = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        gwt::load_issue_monitor_prefs(&prefs_path).expect("load in-flight attempt"),
    );

    assert!(daemon
        .apply_pending_autonomous_handoffs("2026-08-06T06:00:04Z")
        .is_empty());
    assert!(matches!(
        daemon.autonomous_handoffs()[0].delivery,
        AutonomousHandoffDeliveryState::Attempting { attempt: 1, .. }
    ));
}

/// A live gwt process is not enough: once the exact target window fails, its
/// unresolved submit fence must converge to NeedsHuman instead of InFlight.
#[test]
fn target_window_exit_before_receipt_reconciles_as_ambiguous() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    monitor.complete_active_launch(42, "tab-1::agent-answer");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist target");
    let prepared = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:02Z",
    )
    .expect("prepare")
    .expect("ready")
    {
        AutonomousHandoffDeliveryPreparation::Ready(prepared) => prepared,
        other => panic!("expected Ready, got {other:?}"),
    };
    bind_answer_delivery(&prefs_path, &prepared);
    let mut daemon = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        gwt::load_issue_monitor_prefs(&prefs_path).expect("load bound attempt"),
    );
    assert_eq!(
        daemon.record_agent_window_failed("tab-1::agent-answer", "provider exited"),
        Some(42)
    );

    assert_eq!(
        daemon.apply_pending_autonomous_handoffs("2026-08-06T06:00:04Z"),
        vec![42]
    );
    assert!(matches!(
        daemon.autonomous_handoffs()[0].delivery,
        AutonomousHandoffDeliveryState::Ambiguous { .. }
    ));
    assert_eq!(
        daemon.autonomous_record(42).map(|record| record.phase),
        Some(AutonomousPhase::NeedsHuman)
    );
}

/// A durable Launched row cannot outlive the GUI incarnation that owned its
/// submit fence. After a GUI crash, the stale row must not keep the answer in
/// InFlight forever.
#[test]
fn dead_materializer_with_stale_launched_row_reconciles_as_ambiguous() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    monitor.complete_active_launch(42, "tab-1::agent-answer");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist target");
    let prepared = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:02Z",
    )
    .expect("prepare")
    .expect("ready")
    {
        AutonomousHandoffDeliveryPreparation::Ready(prepared) => prepared,
        other => panic!("expected Ready, got {other:?}"),
    };
    bind_answer_delivery(&prefs_path, &prepared);
    let mut crashed = gwt::load_issue_monitor_prefs(&prefs_path).expect("load bound attempt");
    let AutonomousHandoffDeliveryState::Attempting {
        target: Some(target),
        ..
    } = &mut crashed.autonomous_handoffs[0].delivery
    else {
        panic!("expected bound Attempting delivery");
    };
    target.materializer_pid = 2_000_000_000;
    target.materializer_started_at = 1;
    let mut daemon = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), crashed);

    assert_eq!(
        daemon.apply_pending_autonomous_handoffs("2026-08-06T06:00:04Z"),
        vec![42]
    );
    assert!(matches!(
        daemon.autonomous_handoffs()[0].delivery,
        AutonomousHandoffDeliveryState::Ambiguous { .. }
    ));
    assert_eq!(daemon.active_count(), 0);
}

/// Once a worker has started, its error is outcome-ambiguous even when no
/// UserPromptSubmit receipt was observed. Only the exact write-ahead identity
/// may terminally release that delivery's launch ownership.
#[test]
fn worker_started_delivery_error_is_marked_ambiguous_by_exact_attempt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    monitor.complete_active_launch(42, "tab-1::agent-answer");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist prefs");

    let prepared = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:02Z",
    )
    .expect("prepare")
    .expect("ready")
    {
        AutonomousHandoffDeliveryPreparation::Ready(attempt) => attempt,
        other => panic!("expected Ready, got {other:?}"),
    };
    assert!(!gwt::mark_autonomous_handoff_delivery_ambiguous_from_prefs(
        &prefs_path,
        &prepared.handoff_id,
        &prepared.session_id,
        prepared.attempt + 1,
        "wrong attempt",
        "2026-08-06T06:00:03Z",
    )
    .expect("reject stale identity"));
    assert!(gwt::mark_autonomous_handoff_delivery_ambiguous_from_prefs(
        &prefs_path,
        &prepared.handoff_id,
        &prepared.session_id,
        prepared.attempt,
        "worker exited after PTY write started",
        "2026-08-06T06:00:04Z",
    )
    .expect("mark exact attempt ambiguous"));

    let prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load terminal prefs");
    assert!(prefs
        .launched_issues
        .iter()
        .all(|item| item.issue_number != 42));
    assert!(prefs
        .pending_launch_deliveries
        .iter()
        .all(|delivery| delivery.issue_number != 42));
    assert!(prefs.autonomous_records.iter().any(|record| {
        record.issue_number == 42 && record.phase == AutonomousPhase::NeedsHuman
    }));
    assert!(matches!(
        prefs.autonomous_handoffs[0].delivery,
        AutonomousHandoffDeliveryState::Ambiguous { attempt: 1, .. }
    ));
}

/// A definitely-not-submitted failure retries only the answered handoff,
/// keeps ResumeIfSafe, then escalates at the configured bound.
#[test]
fn answered_handoff_delivery_failure_uses_bounded_resume_backoff() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut prefs = IssueMonitorPrefs {
        autonomous_mode: true,
        ..IssueMonitorPrefs::default()
    };
    prefs.autonomous_tuning = gwt::issue_monitor::AutonomousTuning {
        max_attempts: 2,
        retry_backoff_base_secs: 60,
        retry_backoff_cap_secs: 60,
        ..gwt::issue_monitor::AutonomousTuning::default()
    };
    let mut monitor = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Yes", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    monitor.complete_active_launch(42, "tab-1::agent-answer");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist prefs");

    let first = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:02Z",
    )
    .expect("prepare first")
    .expect("first result")
    {
        AutonomousHandoffDeliveryPreparation::Ready(attempt) => attempt,
        other => panic!("expected first Ready, got {other:?}"),
    };
    assert!(matches!(
        gwt::record_autonomous_handoff_delivery_failure_from_prefs(
            &prefs_path,
            &first.handoff_id,
            &first.session_id,
            first.attempt,
            "PTY reservation failed before write",
            "2026-08-06T06:00:03Z",
        )
        .expect("record first failure"),
        AutonomousHandoffDeliveryFailureOutcome::Retry { attempt: 1, .. }
    ));
    let retry_prefs = gwt::load_issue_monitor_prefs(&prefs_path).expect("load retry prefs");
    assert_eq!(
        retry_prefs.queued_launch_session_strategies.get(&42),
        Some(&gwt::IssueMonitorLaunchSessionStrategy::ResumeIfSafe)
    );
    assert!(retry_prefs
        .launched_issues
        .iter()
        .all(|item| item.issue_number != 42));
    assert!(matches!(
        gwt::prepare_autonomous_handoff_delivery_from_prefs(
            &prefs_path,
            42,
            "2026-08-06T06:00:30Z"
        )
        .expect("read backoff")
        .expect("backoff result"),
        AutonomousHandoffDeliveryPreparation::Backoff { attempt: 1, .. }
    ));

    let second = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:02:00Z",
    )
    .expect("prepare second")
    .expect("second result")
    {
        AutonomousHandoffDeliveryPreparation::Ready(attempt) => attempt,
        other => panic!("expected second Ready, got {other:?}"),
    };
    assert_eq!(second.attempt, 2);
    assert!(matches!(
        gwt::record_autonomous_handoff_delivery_failure_from_prefs(
            &prefs_path,
            &second.handoff_id,
            &second.session_id,
            second.attempt,
            "provider rejected resume before submit",
            "2026-08-06T06:02:01Z",
        )
        .expect("record terminal failure"),
        AutonomousHandoffDeliveryFailureOutcome::Escalated { attempt: 2, .. }
    ));
    let terminal = gwt::load_issue_monitor_prefs(&prefs_path).expect("load terminal prefs");
    assert!(terminal
        .failed_issues
        .iter()
        .any(|failed| failed.issue_number == 42));
    assert!(terminal
        .pending_launch_deliveries
        .iter()
        .all(|delivery| delivery.issue_number != 42));
    assert!(matches!(
        terminal.autonomous_handoffs[0].delivery,
        AutonomousHandoffDeliveryState::Exhausted { attempt: 2, .. }
    ));
}

/// A delayed receipt from a previous answer cannot acknowledge a replacement
/// answer because the current canonical prompt hash changed under the lock.
#[test]
fn delayed_receipt_is_rejected_after_the_human_updates_the_answer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prefs_path = dir.path().join("issue-monitor.json");
    let mut monitor = autonomous_monitor();
    let issue = auto_issue(42);
    launch_autonomous(&mut monitor, &issue);
    monitor.absorb_autonomous_handoffs(vec![handoff(42, "session-abc")]);
    monitor.apply_pending_autonomous_handoffs(NOW);
    monitor.answer_autonomous_handoff("handoff-42", "Old answer", "2026-08-06T06:00:00Z");
    monitor.resume_answered_autonomous_handoffs("2026-08-06T06:00:01Z");
    gwt::save_issue_monitor_prefs(&prefs_path, &monitor.prefs()).expect("persist prefs");
    let old = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:00:02Z",
    )
    .expect("prepare old")
    .expect("old ready")
    {
        AutonomousHandoffDeliveryPreparation::Ready(attempt) => attempt,
        other => panic!("expected old Ready, got {other:?}"),
    };
    let old_receipt_identity = bind_answer_delivery(&prefs_path, &old);
    // Restart reconciliation makes the unresolved old attempt human-owned.
    gwt::prepare_autonomous_handoff_delivery_from_prefs(&prefs_path, 42, "2026-08-06T06:01:00Z")
        .expect("reconcile old attempt");
    let mut replacement = IssueMonitorState::with_prefs(
        IssueMonitorConfig::default(),
        gwt::load_issue_monitor_prefs(&prefs_path).expect("load replacement state"),
    );
    assert!(replacement.answer_autonomous_handoff(
        "handoff-42",
        "Replacement answer",
        "2026-08-06T06:02:00Z"
    ));
    replacement.resume_answered_autonomous_handoffs("2026-08-06T06:02:01Z");
    replacement.complete_active_launch(42, "tab-1::agent-answer");
    gwt::save_issue_monitor_prefs(&prefs_path, &replacement.prefs()).expect("save replacement");
    let new = match gwt::prepare_autonomous_handoff_delivery_from_prefs(
        &prefs_path,
        42,
        "2026-08-06T06:02:02Z",
    )
    .expect("prepare replacement")
    .expect("replacement ready")
    {
        AutonomousHandoffDeliveryPreparation::Ready(attempt) => attempt,
        other => panic!("expected replacement Ready, got {other:?}"),
    };
    let new_receipt_identity = bind_answer_delivery(&prefs_path, &new);

    assert!(
        !gwt::acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
            &prefs_path,
            "session-abc",
            &old_receipt_identity,
            &old.prompt,
            "2026-08-06T06:03:00Z",
        )
        .expect("reject stale receipt")
    );
    assert!(
        gwt::acknowledge_autonomous_handoff_user_prompt_submit_from_prefs(
            &prefs_path,
            "session-abc",
            &new_receipt_identity,
            &new.prompt,
            "2026-08-06T06:03:01Z",
        )
        .expect("accept replacement receipt")
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
        restarted
            .next_launch_request("2026-08-06T07:00:01Z")
            .is_none(),
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
