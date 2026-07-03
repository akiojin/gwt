use gwt::{
    BackendEvent, FrontendEvent, IssueMonitorInboxItem, IssueMonitorIssue, IssueMonitorIssueState,
    IssueMonitorLaunchPlan, IssueMonitorStatusView, MonitorInboxState,
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

    // SPEC #3200 T-047: the autonomous-mode toggle wire shape must deserialize.
    let event: FrontendEvent =
        serde_json::from_str(r#"{"kind":"set_issue_monitor_autonomous_mode","enabled":true}"#)
            .expect("autonomous mode enabled event");
    assert!(matches!(
        event,
        FrontendEvent::SetIssueMonitorAutonomousMode { enabled: true }
    ));
    let event: FrontendEvent =
        serde_json::from_str(r#"{"kind":"set_issue_monitor_autonomous_mode","enabled":false}"#)
            .expect("autonomous mode disabled event");
    assert!(matches!(
        event,
        FrontendEvent::SetIssueMonitorAutonomousMode { enabled: false }
    ));

    let event: FrontendEvent =
        serde_json::from_str(r#"{"kind":"issue_monitor_launch_now","issue_number":42}"#)
            .expect("launch now event");
    assert!(matches!(
        event,
        FrontendEvent::IssueMonitorLaunchNow {
            issue_number: 42,
            linked_issue_kind: None
        }
    ));

    let event: FrontendEvent = serde_json::from_str(
        r#"{"kind":"issue_monitor_launch_now","issue_number":3165,"linked_issue_kind":"spec"}"#,
    )
    .expect("configure spec event");
    assert!(matches!(
        event,
        FrontendEvent::IssueMonitorLaunchNow {
            issue_number: 3165,
            linked_issue_kind: Some(gwt::LinkedIssueKind::Spec)
        }
    ));

    let event: FrontendEvent = serde_json::from_str(
        r#"{"kind":"issue_monitor_configure_issue","issue_number":3165,"linked_issue_kind":"spec"}"#,
    )
    .expect("configure spec event");
    assert!(matches!(
        event,
        FrontendEvent::IssueMonitorConfigureIssue {
            issue_number: 3165,
            linked_issue_kind: Some(gwt::LinkedIssueKind::Spec)
        }
    ));

    let event: FrontendEvent = serde_json::from_str(
        r#"{"kind":"reorder_issue_monitor_issues","issue_numbers":[44,42,43]}"#,
    )
    .expect("reorder event");
    assert!(matches!(
        event,
        FrontendEvent::ReorderIssueMonitorIssues { issue_numbers } if issue_numbers == vec![44, 42, 43]
    ));

    let event: FrontendEvent = serde_json::from_str(
        r#"{"kind":"set_issue_monitor_max_active_agents","max_active_agents":3}"#,
    )
    .expect("max active event");
    assert!(matches!(
        event,
        FrontendEvent::SetIssueMonitorMaxActiveAgents {
            max_active_agents: 3
        }
    ));
}

#[test]
fn backend_issue_monitor_status_serializes_for_monitor_card() {
    let event = BackendEvent::IssueMonitorStatus {
        status: IssueMonitorStatusView {
            enabled: true,
            state: "scanning".to_string(),
            queue_len: 2,
            active_count: 1,
            max_active_agents: 3,
            total_candidates: 8,
            active_issue_number: Some(42),
            last_scan_at: Some("2026-06-23T10:00:00Z".to_string()),
            last_error: None,
            launch_profile_source: gwt::IssueMonitorLaunchProfileSource::LastSettings,
            launch_profile_summary: "codex / gpt-5.5 / high / host".to_string(),
            autonomous_mode: false,
            autonomous_issues: Vec::new(),
        },
    };

    let value = serde_json::to_value(event).expect("serialize status");

    assert_eq!(value["kind"], "issue_monitor_status");
    assert_eq!(value["status"]["enabled"], true);
    assert_eq!(value["status"]["queue_len"], 2);
    assert_eq!(value["status"]["active_count"], 1);
    assert_eq!(value["status"]["max_active_agents"], 3);
    assert_eq!(value["status"]["total_candidates"], 8);
    assert_eq!(value["status"]["active_issue_number"], 42);
    assert_eq!(value["status"]["launch_profile_source"], "last_settings");
    assert_eq!(
        value["status"]["launch_profile_summary"],
        "codex / gpt-5.5 / high / host"
    );
}

#[test]
fn backend_issue_monitor_inbox_and_toast_are_serializable() {
    let item = IssueMonitorInboxItem {
        issue: IssueMonitorIssue {
            number: 42,
            title: "Improve monitor".to_string(),
            labels: vec!["auto-improve".to_string()],
            state: IssueMonitorIssueState::Open,
            body: Some("Issue body".to_string()),
            url: Some("https://github.com/example/repo/issues/42".to_string()),
        },
        state: MonitorInboxState::Queued,
        claim_id: Some("claim-a".to_string()),
        blocked_by_owner: None,
        claim_expires_at: None,
        launched_window_id: None,
        launch_plan: Some(IssueMonitorLaunchPlan {
            branch_name: "work/issue-42".to_string(),
            linked_issue_kind: gwt::LinkedIssueKind::Issue,
            prompt: "$gwt-fix-issue #42".to_string(),
        }),
        error_message: None,
    };
    let inbox = serde_json::to_value(BackendEvent::IssueMonitorInbox { items: vec![item] })
        .expect("serialize inbox");
    let toast = serde_json::to_value(BackendEvent::IssueMonitorToast {
        level: "info".to_string(),
        message: "Issue queued".to_string(),
        issue_number: Some(42),
    })
    .expect("serialize toast");
    let launch_failed = serde_json::to_value(BackendEvent::IssueMonitorLaunchFailed {
        issue_number: 42,
        message: "Launch failed".to_string(),
    })
    .expect("serialize launch failed");

    assert_eq!(inbox["kind"], "issue_monitor_inbox");
    assert_eq!(inbox["items"][0]["issue"]["number"], 42);
    assert_eq!(inbox["items"][0]["state"], "queued");
    assert_eq!(
        inbox["items"][0]["launch_plan"]["branch_name"],
        "work/issue-42"
    );
    assert_eq!(
        inbox["items"][0]["launch_plan"]["prompt"],
        "$gwt-fix-issue #42"
    );
    assert_eq!(inbox["items"][0]["issue"]["body"], "Issue body");
    assert_eq!(
        inbox["items"][0]["issue"]["url"],
        "https://github.com/example/repo/issues/42"
    );
    assert_eq!(toast["kind"], "issue_monitor_toast");
    assert_eq!(toast["issue_number"], 42);
    assert_eq!(launch_failed["kind"], "issue_monitor_launch_failed");
    assert_eq!(launch_failed["issue_number"], 42);
    assert_eq!(launch_failed["message"], "Launch failed");
}
