//! SPEC #2780 protocol contract tests for Release Notes window.

use gwt::{BackendEvent, FrontendEvent};
use gwt_core::release_notes::{ReleaseEntry, Section};
use serde_json::json;

#[test]
fn deserializes_open_release_notes_with_focus_version() {
    let msg = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "open_release_notes",
        "id": "project-1::release-notes-1",
        "focus_version": "9.38.0"
    }))
    .expect("open_release_notes with focus_version should deserialize");

    match msg {
        FrontendEvent::OpenReleaseNotes { id, focus_version } => {
            assert_eq!(id, "project-1::release-notes-1");
            assert_eq!(focus_version.as_deref(), Some("9.38.0"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn deserializes_open_release_notes_without_focus_version() {
    let msg = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "open_release_notes",
        "id": "project-1::release-notes-1"
    }))
    .expect("open_release_notes without focus_version should deserialize");

    match msg {
        FrontendEvent::OpenReleaseNotes { id, focus_version } => {
            assert_eq!(id, "project-1::release-notes-1");
            assert!(focus_version.is_none());
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn serializes_release_notes_payload_with_snake_case_kind() {
    let entries = vec![ReleaseEntry {
        version: "9.38.0".into(),
        date: "2026-05-19".into(),
        sections: vec![Section {
            heading: "Bug Fixes".into(),
            items: vec!["fix something".into()],
        }],
    }];

    let event = BackendEvent::ReleaseNotesPayload {
        id: "project-1::release-notes-1".into(),
        entries,
        focus_version: Some("9.38.0".into()),
        current_version: "9.36.0".into(),
    };

    let v = serde_json::to_value(&event).expect("should serialize");
    assert_eq!(v["kind"], "release_notes_payload");
    assert_eq!(v["id"], "project-1::release-notes-1");
    assert_eq!(v["focus_version"], "9.38.0");
    assert_eq!(v["entries"][0]["version"], "9.38.0");
    assert_eq!(v["entries"][0]["date"], "2026-05-19");
    assert_eq!(v["entries"][0]["sections"][0]["heading"], "Bug Fixes");
    assert_eq!(v["entries"][0]["sections"][0]["items"][0], "fix something");
}

#[test]
fn omits_focus_version_when_none_for_payload() {
    let event = BackendEvent::ReleaseNotesPayload {
        id: "project-1::release-notes-1".into(),
        entries: vec![],
        focus_version: None,
        current_version: "9.36.0".into(),
    };
    let v = serde_json::to_value(&event).expect("should serialize");
    assert!(
        v.get("focus_version").is_none(),
        "focus_version must be omitted when None: {v}"
    );
}

// SPEC #2780 v2 Amendment (FR-013): payload carries the current running
// version so the frontend can label the Update action button correctly.
#[test]
fn serializes_release_notes_payload_with_current_version() {
    let event = BackendEvent::ReleaseNotesPayload {
        id: "project-1::release-notes-1".into(),
        entries: vec![],
        focus_version: None,
        current_version: "9.36.0".into(),
    };
    let v = serde_json::to_value(&event).expect("should serialize");
    assert_eq!(v["current_version"], "9.36.0");
}

// SPEC #2780 v2 Amendment (FR-014): FrontendEvent::ApplyUpdateToVersion
// carries the user's chosen version so the backend can resolve the matching
// release tag.
#[test]
fn deserializes_apply_update_to_version() {
    let msg = serde_json::from_value::<FrontendEvent>(json!({
        "kind": "apply_update_to_version",
        "version": "9.36.0"
    }))
    .expect("apply_update_to_version should deserialize");

    match msg {
        FrontendEvent::ApplyUpdateToVersion { version } => {
            assert_eq!(version, "9.36.0");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn serializes_release_notes_error_with_snake_case_kind() {
    let event = BackendEvent::ReleaseNotesError {
        id: "project-1::release-notes-1".into(),
        message: "Release notes could not be loaded".into(),
    };
    let v = serde_json::to_value(&event).expect("should serialize");
    assert_eq!(v["kind"], "release_notes_error");
    assert_eq!(v["message"], "Release notes could not be loaded");
}
