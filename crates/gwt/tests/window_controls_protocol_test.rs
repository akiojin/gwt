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
}
