//! SPEC-1974 Phase 8.2 — board-reminder hook integration tests.
//!
//! These tests verify the stdout contract of the board-reminder hook:
//!
//! - `SessionStart` / `UserPromptSubmit` emit
//!   `{"hookSpecificOutput": {"hookEventName": "...", "additionalContext": "..."}}`
//!   to stdout.
//! - `Stop` emits `{"systemMessage":"..."}`.
//! - `PreToolUse` / `PostToolUse` are silent (no stdout).
//! - Reminder text carries the DO / DO-NOT guard clauses required by
//!   FR-036.

use gwt::cli::hook::board_reminder;
use serde_json::Value;

fn run(event: &str) -> (String, usize) {
    let mut stdout: Vec<u8> = Vec::new();
    // GWT_SESSION_ID env var is not set in this test process, so the
    // hook should fall through to a silent no-op rather than touching
    // a real session file. This pins the "no session" branch of the
    // public handler.
    let output = board_reminder::handle_with_input(event, "").unwrap();
    output.serialize_to(&mut stdout).unwrap();
    let text = String::from_utf8(stdout).unwrap();
    let lines = text.lines().count();
    (text, lines)
}

#[test]
fn pre_tool_use_emits_nothing() {
    let (text, _) = run("PreToolUse");
    assert!(
        text.is_empty(),
        "PreToolUse must produce no stdout, got: {text}"
    );
}

#[test]
fn post_tool_use_emits_nothing() {
    let (text, _) = run("PostToolUse");
    assert!(
        text.is_empty(),
        "PostToolUse must produce no stdout, got: {text}"
    );
}

#[test]
fn reminder_payload_shape_matches_claude_code_contract() {
    use chrono::Utc;
    use gwt_agent::{AgentId, Session};

    let dir = tempfile::tempdir().unwrap();
    let session = {
        let mut s = Session::new(dir.path(), "feature/test", AgentId::Codex);
        s.display_name = "Codex".to_string();
        s
    };

    let plan = board_reminder::compute_plan("UserPromptSubmit", &session, Utc::now())
        .unwrap()
        .expect("UserPromptSubmit must produce output");
    let mut buf = Vec::new();
    plan.output.serialize_to(&mut buf).unwrap();
    let parsed: Value = serde_json::from_slice(&buf).unwrap();

    assert_eq!(
        parsed["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    let additional = parsed["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(additional.contains("phase"));
    assert!(additional.contains("Do NOT"));
}

#[test]
fn stop_payload_uses_system_message_contract() {
    use chrono::Utc;
    use gwt_agent::{AgentId, Session};

    let dir = tempfile::tempdir().unwrap();
    let session = {
        let mut s = Session::new(dir.path(), "feature/test", AgentId::Codex);
        s.display_name = "Codex".to_string();
        s
    };

    let plan = board_reminder::compute_plan("Stop", &session, Utc::now())
        .unwrap()
        .expect("Stop must produce output");
    let mut buf = Vec::new();
    plan.output.serialize_to(&mut buf).unwrap();
    let parsed: Value = serde_json::from_slice(&buf).unwrap();

    assert!(parsed.get("hookSpecificOutput").is_none());
    assert!(parsed["systemMessage"].as_str().unwrap().contains("Stop"));
}
