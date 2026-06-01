//! SPEC #2920 Phase 11 — autostart Settings WebSocket contract.

use gwt::{
    protocol::{backend_event_policy, BackendEventBackpressurePolicy, BackendEventDeliveryClass},
    BackendEvent, FrontendEvent,
};
use serde_json::json;

#[test]
fn deserializes_get_autostart_status() {
    let msg = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "get_autostart_status"
    }))
    .expect("get_autostart_status should deserialize");

    assert!(matches!(msg, FrontendEvent::GetAutostartStatus));
}

#[test]
fn deserializes_update_autostart() {
    let msg = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "update_autostart",
        "enabled": true
    }))
    .expect("update_autostart should deserialize");

    match msg {
        FrontendEvent::UpdateAutostart { enabled } => assert!(enabled),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn serializes_autostart_status_with_snake_case_kind() {
    let event = BackendEvent::AutostartStatus {
        enabled: true,
        mechanism: "LaunchAgent".to_string(),
        install_path: Some("/Users/example/Library/LaunchAgents/GWT.plist".to_string()),
    };

    let value = serde_json::to_value(&event).expect("should serialize");
    assert_eq!(value["kind"], "autostart_status");
    assert_eq!(value["enabled"], true);
    assert_eq!(value["mechanism"], "LaunchAgent");
    assert_eq!(
        value["install_path"],
        "/Users/example/Library/LaunchAgents/GWT.plist"
    );
}

#[test]
fn serializes_autostart_error_with_snake_case_kind() {
    let event = BackendEvent::AutostartError {
        message: "autostart is not supported on this OS".to_string(),
    };

    let value = serde_json::to_value(&event).expect("should serialize");
    assert_eq!(value["kind"], "autostart_error");
    assert_eq!(value["message"], "autostart is not supported on this OS");
}

#[test]
fn autostart_status_policy_is_client_scoped_snapshot() {
    let policy = backend_event_policy("autostart_status")
        .expect("autostart_status must have a backend policy");
    assert_eq!(policy.delivery, BackendEventDeliveryClass::Snapshot);
    assert_eq!(
        policy.backpressure,
        BackendEventBackpressurePolicy::ClientScopedSnapshot
    );
}
