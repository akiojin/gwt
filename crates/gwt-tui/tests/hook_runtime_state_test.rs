//! T-020/T-024 (SPEC #1942) — runtime-state hook tests.
//!
//! The runtime-state hook translates Claude Code hook events into a small
//! JSON file at `$GWT_SESSION_RUNTIME_PATH` that the Branches tab polls to
//! render per-session status badges. This test pins:
//!
//! - the event → status mapping (`SessionStart`/`Stop` → `WaitingInput`,
//!   `PreToolUse` → `Running`),
//! - that writes are crash-safe (no `.tmp-*` residue after success),
//! - that the active-file is rewritten, not appended, on repeat calls,
//! - that unknown events surface as `HookError::InvalidEvent`,
//! - that an unset `GWT_SESSION_RUNTIME_PATH` turns the handler into a
//!   no-op (no panic, no error).

use std::fs;

use gwt_tui::cli::hook::runtime_state::{self, RuntimeState};
use gwt_tui::cli::hook::HookError;

#[test]
fn write_for_event_pretooluse_maps_to_running() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runtime-state.json");

    runtime_state::write_for_event(&path, "PreToolUse").expect("write should succeed");

    let raw = fs::read_to_string(&path).unwrap();
    let state: RuntimeState = serde_json::from_str(&raw).unwrap();
    assert_eq!(state.status, "Running");
    assert_eq!(state.source_event, "PreToolUse");
    assert!(!state.updated_at.is_empty(), "updated_at must be populated");
    assert_eq!(
        state.updated_at, state.last_activity_at,
        "legacy Node contract: both timestamps are the same wall clock"
    );
}

#[test]
fn write_for_event_stop_maps_to_waiting_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runtime-state.json");

    runtime_state::write_for_event(&path, "Stop").expect("write should succeed");

    let raw = fs::read_to_string(&path).unwrap();
    let state: RuntimeState = serde_json::from_str(&raw).unwrap();
    assert_eq!(state.status, "WaitingInput");
    assert_eq!(state.source_event, "Stop");
}

#[test]
fn write_for_event_session_start_maps_to_waiting_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runtime-state.json");

    runtime_state::write_for_event(&path, "SessionStart").expect("write should succeed");

    let raw = fs::read_to_string(&path).unwrap();
    let state: RuntimeState = serde_json::from_str(&raw).unwrap();
    assert_eq!(state.status, "WaitingInput");
    assert_eq!(state.source_event, "SessionStart");
}

#[test]
fn repeated_writes_overwrite_and_leave_no_tmp_residue() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runtime-state.json");

    runtime_state::write_for_event(&path, "PreToolUse").unwrap();
    runtime_state::write_for_event(&path, "Stop").unwrap();
    runtime_state::write_for_event(&path, "PreToolUse").unwrap();

    // Only the canonical file should exist — no `.tmp-*` siblings.
    let mut entries: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, vec!["runtime-state.json".to_string()]);

    // And the *latest* write wins.
    let raw = fs::read_to_string(&path).unwrap();
    let state: RuntimeState = serde_json::from_str(&raw).unwrap();
    assert_eq!(state.status, "Running");
    assert_eq!(state.source_event, "PreToolUse");
}

#[test]
fn unknown_event_surfaces_as_invalid_event_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runtime-state.json");

    let err = runtime_state::write_for_event(&path, "NotARealEvent")
        .expect_err("unknown event must be an error");

    match err {
        HookError::InvalidEvent(name) => assert_eq!(name, "NotARealEvent"),
        other => panic!("expected InvalidEvent, got {other:?}"),
    }

    // And the file must not have been created.
    assert!(
        !path.exists(),
        "an invalid event must not leave a partial state file"
    );
}

#[test]
fn handle_is_noop_when_env_var_is_unset() {
    // SAFETY: this test manipulates a process-global env var. It runs in
    // the same binary as other `hook_runtime_state_test` cases but none
    // of them look at the env var, so there is no cross-test interaction.
    // We use a short helper that saves/restores the prior value.
    let prev = std::env::var_os("GWT_SESSION_RUNTIME_PATH");
    std::env::remove_var("GWT_SESSION_RUNTIME_PATH");

    let result = runtime_state::handle("PreToolUse");

    if let Some(v) = prev {
        std::env::set_var("GWT_SESSION_RUNTIME_PATH", v);
    }

    assert!(
        result.is_ok(),
        "missing GWT_SESSION_RUNTIME_PATH must be a silent no-op, got {result:?}"
    );
}
