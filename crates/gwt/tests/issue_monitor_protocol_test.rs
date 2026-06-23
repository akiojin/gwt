use gwt::{
    BackendEvent, FrontendEvent, IssueMonitorInboxItem, IssueMonitorIssue, IssueMonitorIssueState,
    IssueMonitorStatusView, MonitorInboxState,
};

#[test]
fn frontend_issue_monitor_events_use_snake_case_wire_shape() {
    let event: FrontendEvent =
        serde_json::from_str(r#"{"kind":"set_issue_monitor_enabled","enabled":true}"#)
            .expect("set enabled event");
    assert!(matches!(
        event,
        FrontendEvent::SetIssueMonitorEnabled { enabled: true }
    ));

    let event: FrontendEvent =
        serde_json::from_str(r#"{"kind":"issue_monitor_launch_now","issue_number":42}"#)
            .expect("launch now event");
    assert!(matches!(
        event,
        FrontendEvent::IssueMonitorLaunchNow { issue_number: 42 }
    ));
}

#[test]
fn backend_issue_monitor_status_serializes_for_monitor_card() {
    let event = BackendEvent::IssueMonitorStatus {
        status: IssueMonitorStatusView {
            enabled: true,
            state: "scanning".to_string(),
            queue_len: 2,
            active_issue_number: Some(42),
            last_scan_at: Some("2026-06-23T10:00:00Z".to_string()),
            last_error: None,
        },
    };

    let value = serde_json::to_value(event).expect("serialize status");

    assert_eq!(value["kind"], "issue_monitor_status");
    assert_eq!(value["status"]["enabled"], true);
    assert_eq!(value["status"]["queue_len"], 2);
    assert_eq!(value["status"]["active_issue_number"], 42);
}

#[test]
fn backend_issue_monitor_inbox_and_toast_are_serializable() {
    let item = IssueMonitorInboxItem {
        issue: IssueMonitorIssue {
            number: 42,
            title: "Improve monitor".to_string(),
            labels: vec!["auto-improve".to_string()],
            state: IssueMonitorIssueState::Open,
        },
        state: MonitorInboxState::Queued,
        claim_id: Some("claim-a".to_string()),
        blocked_by_owner: None,
        claim_expires_at: None,
        launched_window_id: None,
    };
    let inbox = serde_json::to_value(BackendEvent::IssueMonitorInbox { items: vec![item] })
        .expect("serialize inbox");
    let toast = serde_json::to_value(BackendEvent::IssueMonitorToast {
        level: "info".to_string(),
        message: "Issue queued".to_string(),
        issue_number: Some(42),
    })
    .expect("serialize toast");

    assert_eq!(inbox["kind"], "issue_monitor_inbox");
    assert_eq!(inbox["items"][0]["issue"]["number"], 42);
    assert_eq!(inbox["items"][0]["state"], "queued");
    assert_eq!(toast["kind"], "issue_monitor_toast");
    assert_eq!(toast["issue_number"], 42);
}
