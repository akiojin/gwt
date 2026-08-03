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
use std::{ffi::OsString, path::Path, sync::Mutex};

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn with_isolated_home<T>(run: impl FnOnce(&Path) -> T) -> T {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = tempfile::tempdir().expect("home");
    let _home = ScopedEnvVar::set("HOME", home.path());
    let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
    run(home.path())
}

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

    with_isolated_home(|_| {
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
        assert!(additional.contains("Do NOT") || additional.contains("手動で作成"));
    });
}

#[test]
fn stop_payload_uses_system_message_contract() {
    use chrono::Utc;
    use gwt_agent::{AgentId, Session};

    with_isolated_home(|_| {
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
    });
}
