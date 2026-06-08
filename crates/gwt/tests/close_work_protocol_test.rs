use gwt::FrontendEvent;
use serde_json::json;

// SPEC-2359 Phase W-12 Slice 4 (FR-352): the Work surface Done / Discard
// buttons send `{kind: "close_work", work_id, close_kind}`. Verify the backend
// protocol deserializes both close kinds so the wire contract with
// `web/workspace-kanban-surface.js` stays intact.
#[test]
fn frontend_event_deserializes_close_work_done() {
    let event = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "close_work",
        "work_id": "work-session-session-a",
        "close_kind": "done"
    }))
    .expect("close_work done should deserialize");
    match event {
        FrontendEvent::CloseWork {
            work_id,
            close_kind,
        } => {
            assert_eq!(work_id, "work-session-session-a");
            assert_eq!(close_kind, "done");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn frontend_event_deserializes_close_work_discarded() {
    let event = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "close_work",
        "work_id": "work-session-session-b",
        "close_kind": "discarded"
    }))
    .expect("close_work discarded should deserialize");
    match event {
        FrontendEvent::CloseWork {
            work_id,
            close_kind,
        } => {
            assert_eq!(work_id, "work-session-session-b");
            assert_eq!(close_kind, "discarded");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
