use gwt::FrontendEvent;
use serde_json::json;

#[test]
fn frontend_event_deserializes_window_state_commands() {
    let maximize = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "maximize_window",
        "id": "project-1::claude-1",
        "bounds": {
            "x": 12.0,
            "y": 24.0,
            "width": 1280.0,
            "height": 720.0
        }
    }))
    .expect("maximize_window should deserialize");
    match maximize {
        FrontendEvent::MaximizeWindow { id, bounds } => {
            assert_eq!(id, "project-1::claude-1");
            assert_eq!(bounds.x, 12.0);
            assert_eq!(bounds.y, 24.0);
            assert_eq!(bounds.width, 1280.0);
            assert_eq!(bounds.height, 720.0);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let minimize = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "minimize_window",
        "id": "project-1::claude-1"
    }))
    .expect("minimize_window should deserialize");
    match minimize {
        FrontendEvent::MinimizeWindow { id } => {
            assert_eq!(id, "project-1::claude-1");
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let restore = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "restore_window",
        "id": "project-1::claude-1"
    }))
    .expect("restore_window should deserialize");
    match restore {
        FrontendEvent::RestoreWindow { id } => {
            assert_eq!(id, "project-1::claude-1");
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let list = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "list_windows"
    }))
    .expect("list_windows should deserialize");
    match list {
        FrontendEvent::ListWindows => {}
        other => panic!("unexpected event: {other:?}"),
    }

    let knowledge = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "load_knowledge_bridge",
        "id": "project-1::issue-1",
        "knowledge_kind": "issue",
        "request_id": 7,
        "selected_number": 2017,
        "refresh": true,
        "list_scope": "closed"
    }))
    .expect("load_knowledge_bridge should deserialize");
    match knowledge {
        FrontendEvent::LoadKnowledgeBridge {
            id,
            knowledge_kind,
            request_id,
            selected_number,
            refresh,
            list_scope,
        } => {
            assert_eq!(id, "project-1::issue-1");
            assert_eq!(knowledge_kind, gwt::KnowledgeKind::Issue);
            assert_eq!(request_id, Some(7));
            assert_eq!(selected_number, Some(2017));
            assert!(refresh);
            assert_eq!(list_scope, Some(gwt::KnowledgeListScope::Closed));
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let launch = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "open_issue_launch_wizard",
        "id": "project-1::issue-1",
        "issue_number": 2017
    }))
    .expect("open_issue_launch_wizard should deserialize");
    match launch {
        FrontendEvent::OpenIssueLaunchWizard { id, issue_number } => {
            assert_eq!(id, "project-1::issue-1");
            assert_eq!(issue_number, 2017);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
