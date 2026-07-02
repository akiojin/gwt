use gwt::issue_monitor::{
    github_auth_setup_message, is_auto_improve_candidate, issue_monitor_launch_prompt,
    load_issue_monitor_prefs, save_issue_monitor_prefs, scan_issue_monitor_candidates,
    IssueMonitorConfig, IssueMonitorFailedIssue, IssueMonitorIssue, IssueMonitorIssueState,
    IssueMonitorPrefs, IssueMonitorState, MonitorInboxState,
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
        body: Some(format!("Body {number}")),
        url: Some(format!("https://github.com/example/repo/issues/{number}")),
    }
}

fn github_issue_number(number: u64, comments: Vec<CommentSnapshot>) -> IssueSnapshot {
    IssueSnapshot {
        number: IssueNumber(number),
        title: format!("Improve monitor {number}"),
        body: String::new(),
        labels: vec!["auto-improve".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("t1"),
        comments,
    }
}

fn github_issue(comments: Vec<CommentSnapshot>) -> IssueSnapshot {
    github_issue_number(42, comments)
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
    assert!(is_auto_improve_candidate(&issue(1, &["bug"]), &config));
    assert!(is_auto_improve_candidate(
        &issue(2, &["auto-improve"]),
        &config
    ));

    let mut closed = issue(3, &["auto-improve"]);
    closed.state = IssueMonitorIssueState::Closed;
    assert!(!is_auto_improve_candidate(&closed, &config));
}

#[test]
fn monitor_maps_gwt_spec_label_to_spec_launch_kind() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(3165, &["gwt-spec"]), "claim-spec");

    let launch = monitor.next_launch_request().expect("spec launch request");

    assert_eq!(launch.issue_number, 3165);
    assert_eq!(launch.linked_issue_kind, LinkedIssueKind::Spec);
    assert_eq!(launch.branch_name, "feature/spec-3165");
    assert_eq!(
        issue_monitor_launch_prompt(launch.linked_issue_kind, launch.issue_number),
        "$gwt-build-spec SPEC-3165"
    );
    assert_eq!(
        monitor
            .inbox_item(3165)
            .and_then(|item| item.launch_plan.as_ref())
            .map(|plan| plan.prompt.as_str()),
        Some("$gwt-build-spec SPEC-3165")
    );
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
    assert_eq!(
        issue_monitor_launch_prompt(first.linked_issue_kind, first.issue_number),
        "$gwt-fix-issue #42"
    );
    assert_eq!(monitor.queue_len(), 1);
    assert!(monitor.next_launch_request().is_none());

    monitor.complete_active_launch(42, "tab::agent-42");
    assert_eq!(monitor.active_count(), 1);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Launched
    );
    assert!(
        monitor.next_launch_request().is_none(),
        "launched work still consumes the configured active capacity"
    );

    monitor.set_max_active_agents(2);
    let second = monitor.next_launch_request().expect("second launch");
    assert_eq!(second.issue_number, 43);
    assert_eq!(second.branch_name, "work/issue-43");
}

#[test]
fn monitor_allows_active_launches_up_to_configured_max() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 2,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["bug"]), "claim-a");
    monitor.record_claimed(issue(43, &["enhancement"]), "claim-b");
    monitor.record_claimed(issue(44, &["question"]), "claim-c");

    let first = monitor.next_launch_request().expect("first launch");
    let second = monitor.next_launch_request().expect("second launch");

    assert_eq!(first.issue_number, 42);
    assert_eq!(second.issue_number, 43);
    assert_eq!(monitor.active_count(), 2);
    assert_eq!(monitor.queue_len(), 1);
    assert!(monitor.next_launch_request().is_none());
}

#[test]
fn monitor_reorders_queued_issues_without_preempting_active_launches() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["bug"]), "claim-a");
    monitor.record_claimed(issue(43, &["enhancement"]), "claim-b");
    monitor.record_claimed(issue(44, &["question"]), "claim-c");
    let active = monitor.next_launch_request().expect("active launch");
    assert_eq!(active.issue_number, 42);

    monitor.reorder_queued_issues(&[44, 43, 42]);

    let next = monitor.next_launch_request();
    assert!(next.is_none(), "active launch should not be preempted");
    monitor.complete_active_launch(42, "tab::agent-42");
    assert!(
        monitor.next_launch_request().is_none(),
        "launched work should not free the active slot until capacity changes or the work stops"
    );
    monitor.set_max_active_agents(2);
    assert_eq!(
        monitor
            .next_launch_request()
            .expect("next launch")
            .issue_number,
        44
    );
}

#[test]
fn monitor_reorders_visible_inbox_to_match_queue_priority() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.record_candidate(issue(42, &["bug"]));
    monitor.record_candidate(issue(43, &["enhancement"]));
    monitor.record_candidate(issue(44, &["question"]));

    monitor.reorder_queued_issues(&[44, 42, 43]);

    let visible_numbers: Vec<u64> = monitor.inbox.iter().map(|item| item.issue.number).collect();
    assert_eq!(visible_numbers, vec![44, 42, 43]);
}

#[test]
fn monitor_prefs_persist_priority_and_max_active_agents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("issue_monitor.json");
    let prefs = IssueMonitorPrefs {
        enabled: true,
        max_active_agents: 3,
        priority_order: vec![44, 42, 43],
        launch_profile: None,
        ..IssueMonitorPrefs::default()
    };

    save_issue_monitor_prefs(&path, &prefs).expect("save prefs");
    let loaded = load_issue_monitor_prefs(&path).expect("load prefs");

    assert_eq!(loaded, prefs);
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
fn failed_launch_marks_inbox_failed_and_clears_active_launch() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    let first = monitor.next_launch_request().expect("launch request");
    assert_eq!(first.issue_number, 42);

    monitor.record_launch_auth_required("2026-06-23T10:02:00Z");
    monitor.record_launch_failed(42, "binary missing");

    assert_eq!(monitor.active_count(), 0);
    assert_eq!(monitor.queue_len(), 0);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::LaunchFailed
    );
    assert_eq!(
        monitor
            .inbox_item(42)
            .expect("inbox item")
            .error_message
            .as_deref(),
        Some("binary missing")
    );
    assert_eq!(
        monitor.status_view().last_error.as_deref(),
        Some("issue #42: binary missing")
    );
    assert_eq!(monitor.status_view().state, "error");
}

#[test]
fn agent_runtime_failure_marks_launched_issue_failed_and_persists_error() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 2,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    monitor.record_claimed(issue(43, &["bug"]), "claim-b");
    let first = monitor.next_launch_request().expect("launch request");
    assert_eq!(first.issue_number, 42);
    monitor.complete_active_launch(42, "tab::agent-42");

    let failed_issue =
        monitor.record_agent_window_failed("tab::agent-42", "Stop-block hit an error");

    assert_eq!(failed_issue, Some(42));
    assert_eq!(monitor.active_count(), 0);
    assert_eq!(monitor.queue_len(), 1);
    let item = monitor.inbox_item(42).expect("failed inbox item");
    assert_eq!(item.state, MonitorInboxState::AgentFailed);
    assert_eq!(item.launched_window_id, None);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
    assert_eq!(monitor.status_view().state, "error");
    assert_eq!(
        monitor.status_view().last_error.as_deref(),
        Some("issue #42: Stop-block hit an error")
    );
    assert_eq!(
        monitor.prefs().failed_issues,
        vec![IssueMonitorFailedIssue {
            issue_number: 42,
            message: "Stop-block hit an error".to_string(),
            // #3165 error-window lifecycle: the failed agent window id is
            // retained so an explicit Launch Now can close the stale window.
            window_id: Some("tab::agent-42".to_string()),
        }]
    );

    let mut restored =
        IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
    scan_issue_monitor_candidates(
        &mut restored,
        &[issue(42, &["bug"])],
        "2026-06-23T10:03:00Z",
    );
    let restored_item = restored.inbox_item(42).expect("restored failed item");
    assert_eq!(restored_item.state, MonitorInboxState::AgentFailed);
    assert_eq!(
        restored_item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
    assert_eq!(restored.queue_len(), 0);
    assert_eq!(restored.status_view().state, "error");
}

#[test]
fn agent_runtime_failure_matches_raw_and_combined_window_ids() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 2,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    monitor.next_launch_request().expect("launch request");
    monitor.complete_active_launch(42, "tab-1::agent-42");

    let failed_issue = monitor.record_agent_window_failed("agent-42", "Stop-block hit an error");

    assert_eq!(failed_issue, Some(42));
    let item = monitor.inbox_item(42).expect("failed inbox item");
    assert_eq!(item.state, MonitorInboxState::AgentFailed);
    assert_eq!(
        item.error_message.as_deref(),
        Some("Stop-block hit an error")
    );
}

#[test]
fn max_active_increase_allows_more_queued_launches_without_rescan() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 1,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    monitor.record_claimed(issue(42, &["auto-improve"]), "claim-a");
    monitor.record_claimed(issue(43, &["auto-improve"]), "claim-b");
    monitor.record_claimed(issue(44, &["auto-improve"]), "claim-c");

    let first = monitor.next_launch_request().expect("first launch");
    assert_eq!(first.issue_number, 42);
    assert!(monitor.next_launch_request().is_none());

    monitor.set_max_active_agents(3);

    let second = monitor
        .next_launch_request()
        .expect("second launch after max increase");
    let third = monitor
        .next_launch_request()
        .expect("third launch after max increase");
    assert_eq!(second.issue_number, 43);
    assert_eq!(third.issue_number, 44);
    assert_eq!(monitor.active_count(), 3);
    assert_eq!(monitor.queue_len(), 0);
}

#[test]
fn claim_capacity_cap_prevents_multiple_preconfigured_launches() {
    let client = FakeIssueClient::new();
    client.seed(github_issue_number(42, vec![]));
    client.seed(github_issue_number(43, vec![]));
    client.seed(github_issue_number(44, vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 3,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    scan_issue_monitor_candidates(
        &mut monitor,
        &[
            issue(42, &["bug"]),
            issue(43, &["enhancement"]),
            issue(44, &["question"]),
        ],
        "2026-06-23T10:01:00Z",
    );

    let launches = monitor.claim_next_launch_requests_with_active_cap(
        &client,
        "host-a/session-a",
        "2026-06-23T10:01:00Z",
        1,
    );
    let repeated = monitor.claim_next_launch_requests_with_active_cap(
        &client,
        "host-a/session-a",
        "2026-06-23T10:02:00Z",
        1,
    );

    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].issue_number, 42);
    assert!(repeated.is_empty());
    assert_eq!(monitor.active_count(), 1);
    assert_eq!(monitor.queue_len(), 2);
}

#[test]
fn claim_capacity_zero_keeps_queue_visible_without_claiming_or_launching() {
    let client = FakeIssueClient::new();
    client.seed(github_issue_number(42, vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        max_active: 5,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    scan_issue_monitor_candidates(&mut monitor, &[issue(42, &["bug"])], "2026-06-23T10:01:00Z");

    let launches = monitor.claim_next_launch_requests_with_active_cap(
        &client,
        "host-a/session-a",
        "2026-06-23T10:01:00Z",
        0,
    );

    assert!(launches.is_empty());
    assert_eq!(monitor.active_count(), 0);
    assert_eq!(monitor.queue_len(), 1);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
    assert!(
        client.comments(IssueNumber(42)).is_empty(),
        "capacity 0 must not create a GitHub claim"
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
fn scan_candidates_queues_all_open_issues_without_claiming_at_scan_time() {
    let client = FakeIssueClient::new();
    client.seed(github_issue(vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    let summary = scan_issue_monitor_candidates(
        &mut monitor,
        &[issue(42, &["auto-improve"]), issue(43, &["bug"])],
        "2026-06-23T10:00:00Z",
    );

    assert_eq!(summary.claimed, 0);
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.skipped, 0);
    assert!(client.comments(IssueNumber(42)).is_empty());
    assert_eq!(monitor.queue_len(), 2);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn scan_candidates_ignores_claims_until_launch_time() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });

    let summary = scan_issue_monitor_candidates(
        &mut monitor,
        &[issue(42, &["auto-improve"])],
        "2026-06-23T10:01:00Z",
    );

    assert_eq!(summary.blocked, 0);
    assert_eq!(monitor.queue_len(), 1);
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn scan_error_keeps_visible_queue_candidates() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    scan_issue_monitor_candidates(&mut monitor, &[issue(42, &["bug"])], "2026-06-23T10:01:00Z");

    monitor.record_scan_error(
        "2026-06-23T10:02:00Z",
        "GitHub auth failed: test auth failure",
    );

    let status = monitor.status_view();
    assert_eq!(status.queue_len, 1);
    assert_eq!(status.total_candidates, 1);
    assert_eq!(
        status.last_error.as_deref(),
        Some("GitHub auth failed: test auth failure")
    );
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn launch_auth_required_keeps_visible_queue_with_blocking_error() {
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    scan_issue_monitor_candidates(&mut monitor, &[issue(42, &["bug"])], "2026-06-23T10:01:00Z");

    monitor.record_launch_auth_required("2026-06-23T10:02:00Z");

    let status = monitor.status_view();
    assert_eq!(status.state, "error");
    assert_eq!(status.queue_len, 1);
    assert_eq!(status.total_candidates, 1);
    assert_eq!(
        status.last_error.as_deref(),
        Some(github_auth_setup_message())
    );
    let message = status.last_error.as_deref().expect("auth help message");
    assert!(message.contains("gh auth login --hostname github.com"));
    assert!(message.contains("gh auth setup-git"));
    assert!(message.contains("git ls-remote origin HEAD"));
    assert_eq!(
        monitor.inbox_item(42).expect("inbox item").state,
        MonitorInboxState::Queued
    );
}

#[test]
fn monitor_claims_queue_head_just_in_time_and_skips_blocked_claims() {
    let client = FakeIssueClient::new();
    client.seed(github_issue_number(
        42,
        vec![claim_comment("host-b/session-b")],
    ));
    client.seed(github_issue_number(43, vec![]));
    let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
        enabled: true,
        ..IssueMonitorConfig::default()
    });
    monitor.set_gui_connected(true);
    scan_issue_monitor_candidates(
        &mut monitor,
        &[issue(42, &["bug"]), issue(43, &["enhancement"])],
        "2026-06-23T10:01:00Z",
    );
    assert!(client.comments(IssueNumber(43)).is_empty());

    let launches =
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:01:00Z");

    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].issue_number, 43);
    assert_eq!(
        monitor.inbox_item(42).expect("blocked item").state,
        MonitorInboxState::BlockedByClaim
    );
    assert_eq!(
        monitor.inbox_item(43).expect("launched item").state,
        MonitorInboxState::Launching
    );
    assert_eq!(client.comments(IssueNumber(43)).len(), 1);
}
