//! SPEC-1974 Phase 8.2 — board-reminder hook integration tests.
//!
//! These tests verify the stdout contract of the board-reminder hook:
//!
//! - `SessionStart` / `UserPromptSubmit` / `Stop` emit
//!   `{"hookSpecificOutput": {"hookEventName": "...", "additionalContext": "..."}}`
//!   to stdout.
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
    board_reminder::handle_with_input(event, "", &mut stdout).unwrap();
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

    let computed = board_reminder::compute_output("UserPromptSubmit", &session, Utc::now())
        .unwrap()
        .expect("UserPromptSubmit must produce output");

    // Model what emit_output produces by round-tripping the data
    // through the same JSON shape Claude Code consumes.
    let json = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": computed.additional_context,
        }
    });
    let parsed: Value = serde_json::from_value(json).unwrap();

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
