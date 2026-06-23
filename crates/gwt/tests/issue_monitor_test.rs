use gwt::issue_monitor::{
    is_auto_improve_candidate, scan_issue_monitor_candidates, IssueMonitorConfig,
    IssueMonitorIssue, IssueMonitorIssueState, IssueMonitorState, MonitorInboxState,
};
use gwt_github::issue_auto_claim::{render_claim_comment, ClaimComment, ClaimStatus};
use gwt_github::{
    CommentId, CommentSnapshot, FakeIssueClient, IssueNumber, IssueSnapshot, IssueState, UpdatedAt,
};

fn issue(number: u64, labels: &[&str]) -> IssueMonitorIssue {
    IssueMonitorIssue {
        number,
        title: format!("Issue {number}"),
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
        state: IssueMonitorIssueState::Open,
    }
}

fn github_issue(comments: Vec<CommentSnapshot>) -> IssueSnapshot {
    IssueSnapshot {
        number: IssueNumber(42),
        title: "Improve monitor".to_string(),
        body: String::new(),
        labels: vec!["auto-improve".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("t1"),
        comments,
    }
}

fn claim_comment(owner: &str) -> CommentSnapshot {
    let claim = ClaimComment {
        comment_id: Some(CommentId(9)),
        claim_id: "claim-other".to_string(),
        owner: owner.to_string(),
        issue_number: 42,
        status: ClaimStatus::Active,
        heartbeat_at: "2026-06-23T10:00:00Z".to_string(),
        expires_at: "2026-06-23T10:30:00Z".to_string(),
        launched_work_id: Some("work/issue-42".to_string()),
    };
    CommentSnapshot {
        id: CommentId(9),
        body: render_claim_comment(&claim),
        updated_at: UpdatedAt::new("t1"),
    }
}

#[test]
fn monitor_config_defaults_to_disabled_and_label_gates_candidates() {
    let config = IssueMonitorConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.trigger_label, "auto-improve");
    assert!(is_auto_improve_candidate(
        &issue(1, &["bug", "auto-improve"]),
        &config
    ));
    assert!(!is_auto_improve_candidate(&issue(2, &["bug"]), &config));

    let mut closed = issue(3, &["auto-improve"]);
    closed.state = IssueMonitorIssueState::Closed;
    assert!(!is_auto_improve_candidate(&closed, &config));
}

#[test]
fn claimed_issue_waits_in_queue_until_gui_is_connected() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");

    assert_eq!(monitor.queue_len(), 1);
    assert!(monitor.next_launch_request().is_none());
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn monitor_runs_one_active_launch_and_keeps_remaining_items_queued() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    monitor.record_claimed(issue(43, &["auto-improve"]), "claim-b");

    let first = monitor.next_launch_request().expect("first launch");
    assert_eq!(first.issue_number, 42);
    assert_eq!(first.branch_name, "work/issue-42");
    assert_eq!(monitor.queue_len(), 1);
    assert!(monitor.next_launch_request().is_none());

    monitor.complete_active_launch(42, "tab::agent-42");
    let second = monitor.next_launch_request().expect("second launch");
    assert_eq!(second.issue_number, 43);
    assert_eq!(second.branch_name, "work/issue-43");
}

#[test]
fn claimed_active_launch_stays_launching_when_scan_refreshes_claim() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    let first = monitor.next_launch_request().expect("launch request");
    assert_eq!(first.issue_number, 42);

    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a-refresh");

    assert_eq!(monitor.queue_len(), 0);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Launching
    );
}

#[test]
fn blocked_claim_is_visible_in_inbox_without_queueing_launch() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    monitor.record_blocked_by_claim(
        issue(42, &["auto-improve"]),
        "other-host/session",
        "2026-06-23T10:30:00Z",
    );

    let item = monitor.inbox_item(42).expect("inbox item");
    assert_eq!(item.state, MonitorInboxState::BlockedByClaim);
    assert_eq!(item.blocked_by_owner.as_deref(), Some("other-host/session"));
    assert_eq!(
        item.claim_expires_at.as_deref(),
        Some("2026-06-23T10:30:00Z")
    );
    assert_eq!(monitor.queue_len(), 0);
}

#[test]
fn scan_candidates_claims_matching_label_and_queues_issue() {
    let client = FakeIssueClient::new();
    client.seed(github_issue(vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    let summary = scan_issue_monitor_candidates(
        &mut monitor,
        &client,
        &[issue(42, &["auto-improve"]), issue(43, &["bug"])],
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
    );

    assert_eq!(summary.claimed, 1);
    assert_eq!(summary.skipped, 1);
    assert_eq!(monitor.queue_len(), 1);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn scan_candidates_records_blocked_claim_without_queueing() {
    let client = FakeIssueClient::new();
    client.seed(github_issue(vec![claim_comment("host-b/session-b")]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    let summary = scan_issue_monitor_candidates(
        &mut monitor,
        &client,
        &[issue(42, &["auto-improve"])],
        "host-a/session-a",
        "2026-06-23T10:01:00Z",
    );

    assert_eq!(summary.blocked, 1);
    assert_eq!(monitor.queue_len(), 0);
    let item = monitor.inbox_item(42).expect("inbox item");
    assert_eq!(item.state, MonitorInboxState::BlockedByClaim);
    assert_eq!(item.blocked_by_owner.as_deref(), Some("host-b/session-b"));
}
