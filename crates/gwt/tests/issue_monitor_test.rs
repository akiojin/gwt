use gwt::issue_monitor::{
    is_issue_monitor_candidate, load_issue_monitor_settings_from_path,
    prepare_issue_monitor_launches, save_issue_monitor_settings_to_path,
    scan_issue_monitor_candidates, IssueMonitorConfig, IssueMonitorIssue, IssueMonitorIssueState,
    IssueMonitorProjectSettings, IssueMonitorState, MonitorInboxState,
};
use gwt::LinkedIssueKind;
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

fn github_issue(number: u64, labels: &[&str], comments: Vec<CommentSnapshot>) -> IssueSnapshot {
    IssueSnapshot {
        number: IssueNumber(number),
        title: format!("Issue {number}"),
        body: String::new(),
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
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
fn monitor_config_defaults_to_disabled_and_accepts_all_open_issues() {
    let config = IssueMonitorConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.max_active_agents, 1);
    assert!(config.priority_order.is_empty());
    assert!(is_issue_monitor_candidate(&issue(1, &["bug"]), &config));
    assert!(is_issue_monitor_candidate(
        &issue(2, &["gwt-spec"]),
        &config
    ));

    let mut closed = issue(3, &["auto-improve"]);
    closed.state = IssueMonitorIssueState::Closed;
    assert!(!is_issue_monitor_candidate(&closed, &config));
}

#[test]
fn scan_queues_all_open_issues_without_claiming_them() {
    let client = FakeIssueClient::new();
    client.seed(github_issue(42, &["bug"], vec![]));
    client.seed(github_issue(43, &["gwt-spec"], vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    let mut closed = issue(44, &["auto-improve"]);
    closed.state = IssueMonitorIssueState::Closed;
    let summary = scan_issue_monitor_candidates(
        &mut monitor,
        &[issue(42, &["bug"]), issue(43, &["gwt-spec"]), closed],
        "2026-06-23T10:00:00Z",
    );

    assert_eq!(summary.scanned, 3);
    assert_eq!(summary.queued, 2);
    assert_eq!(summary.skipped, 1);
    assert_eq!(monitor.queued_issue_numbers(), vec![42, 43]);
    assert_eq!(client.call_log(), Vec::<String>::new());
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
    assert_eq!(
        monitor
            .inbox_item(43)
            .expect("inbox item")
            .issue
            .linked_issue_kind(),
        LinkedIssueKind::Spec
    );
}

#[test]
fn priority_order_controls_queue_order_and_persists_to_settings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let settings_path = temp.path().join("issue-monitor.json");
    let settings = IssueMonitorProjectSettings {
        enabled: true,
        priority_order: vec![43, 42],
        max_active_agents: 2,
    };
    save_issue_monitor_settings_to_path(&settings_path, &settings).expect("save settings");
    let loaded = load_issue_monitor_settings_from_path(&settings_path).expect("load settings");
    assert_eq!(loaded, settings);

    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        priority_order: loaded.priority_order,
        max_active_agents: loaded.max_active_agents,
        ..IssueMonitorConfig::default()
    });
    scan_issue_monitor_candidates(
        &mut monitor,
        &[
            issue(42, &["bug"]),
            issue(43, &["bug"]),
            issue(44, &["bug"]),
        ],
        "2026-06-23T10:00:00Z",
    );

    assert_eq!(monitor.queued_issue_numbers(), vec![43, 42, 44]);
}

#[test]
fn launch_preparation_acquires_claims_just_in_time_and_respects_max_active_agents() {
    let client = FakeIssueClient::new();
    client.seed(github_issue(42, &["bug"], vec![]));
    client.seed(github_issue(43, &["gwt-spec"], vec![]));
    client.seed(github_issue(44, &["bug"], vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active_agents: 2,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    scan_issue_monitor_candidates(
        &mut monitor,
        &[
            issue(42, &["bug"]),
            issue(43, &["gwt-spec"]),
            issue(44, &["bug"]),
        ],
        "2026-06-23T10:00:00Z",
    );

    let batch = prepare_issue_monitor_launches(
        &mut monitor,
        &client,
        "host-a/session-a",
        "2026-06-23T10:01:00Z",
    );

    assert_eq!(batch.requests.len(), 2);
    assert_eq!(batch.requests[0].issue_number, 42);
    assert_eq!(batch.requests[0].linked_issue_kind, LinkedIssueKind::Issue);
    assert_eq!(batch.requests[0].branch_name, "work/issue-42");
    assert_eq!(batch.requests[1].issue_number, 43);
    assert_eq!(batch.requests[1].linked_issue_kind, LinkedIssueKind::Spec);
    assert_eq!(batch.requests[1].branch_name, "feature/spec-43");
    assert_eq!(monitor.active_issue_numbers(), vec![42, 43]);
    assert_eq!(monitor.queued_issue_numbers(), vec![44]);
    assert!(client
        .call_log()
        .contains(&"create_comment:#42".to_string()));
    assert!(client
        .call_log()
        .contains(&"create_comment:#43".to_string()));
    assert!(!client
        .call_log()
        .contains(&"create_comment:#44".to_string()));
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
        issue(42, &["bug"]),
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
fn blocked_claim_is_recorded_and_launch_preparation_continues_to_next_issue() {
    let client = FakeIssueClient::new();
    client.seed(github_issue(
        42,
        &["bug"],
        vec![claim_comment("host-b/session-b")],
    ));
    client.seed(github_issue(43, &["bug"], vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);

    scan_issue_monitor_candidates(
        &mut monitor,
        &[issue(42, &["bug"]), issue(43, &["bug"])],
        "2026-06-23T10:00:00Z",
    );
    let batch = prepare_issue_monitor_launches(
        &mut monitor,
        &client,
        "host-a/session-a",
        "2026-06-23T10:01:00Z",
    );

    assert_eq!(batch.blocked, 1);
    assert_eq!(batch.requests.len(), 1);
    assert_eq!(batch.requests[0].issue_number, 43);
    assert_eq!(monitor.queue_len(), 0);
    let item = monitor.inbox_item(42).expect("inbox item");
    assert_eq!(item.state, MonitorInboxState::BlockedByClaim);
    assert_eq!(item.blocked_by_owner.as_deref(), Some("host-b/session-b"));
}
