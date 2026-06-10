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
use gwt_agent::GWT_SESSION_ID_ENV;
use serde_json::Value;
use std::{
    ffi::OsString,
    path::Path,
    sync::{Mutex, OnceLock},
};
use tempfile::TempDir;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
    previous_home: Option<OsString>,
    previous_userprofile: Option<OsString>,
    previous_session_id: Option<OsString>,
}

impl EnvGuard {
    fn isolate(home: &Path) -> Self {
        let guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous_home = std::env::var_os("HOME");
        let previous_userprofile = std::env::var_os("USERPROFILE");
        let previous_session_id = std::env::var_os(GWT_SESSION_ID_ENV);
        std::env::set_var("HOME", home);
        std::env::set_var("USERPROFILE", home);
        std::env::remove_var(GWT_SESSION_ID_ENV);
        Self {
            _guard: guard,
            previous_home,
            previous_userprofile,
            previous_session_id,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous_home.take() {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match self.previous_userprofile.take() {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        match self.previous_session_id.take() {
            Some(value) => std::env::set_var(GWT_SESSION_ID_ENV, value),
            None => std::env::remove_var(GWT_SESSION_ID_ENV),
        }
    }
}

fn with_isolated_env<T>(f: impl FnOnce() -> T) -> T {
    let home = TempDir::new().unwrap();
    let _env = EnvGuard::isolate(home.path());
    f()
}

fn run(event: &str) -> (String, usize) {
    with_isolated_env(|| {
        let mut stdout: Vec<u8> = Vec::new();
        // GWT_SESSION_ID is unset in the isolated test process env, so the
        // hook falls through to a silent no-op instead of touching a real
        // developer session file.
        let output = board_reminder::handle_with_input(event, "").unwrap();
        output.serialize_to(&mut stdout).unwrap();
        let text = String::from_utf8(stdout).unwrap();
        let lines = text.lines().count();
        (text, lines)
    })
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
    with_isolated_env(|| {
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
        assert!(additional.contains("Do NOT") || additional.contains("手動で作成"));
    })
}

#[test]
fn stop_payload_uses_system_message_contract() {
    with_isolated_env(|| {
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
    })
}
