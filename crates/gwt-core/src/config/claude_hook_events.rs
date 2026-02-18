//! Claude Code Hook event processing (SPEC-861d8cdf FR-101)
//!
//! This module processes hook payloads from Claude Code (stdin JSON) and updates
//! gwt session status stored under `~/.gwt/sessions/`.

use crate::error::{GwtError, Result};
use serde_json::Value;
use std::path::PathBuf;

use super::session::{AgentStatus, Session};

fn extract_worktree_path(payload: &Value) -> Option<PathBuf> {
    let cwd = payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("worktree").and_then(|v| v.as_str()))
        .or_else(|| payload.get("worktree_path").and_then(|v| v.as_str()))
        .or_else(|| payload.get("project").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())?;

    Some(PathBuf::from(cwd))
}

fn extract_notification_type(payload: &Value) -> Option<&str> {
    payload.get("notification_type").and_then(|v| v.as_str())
}

fn status_for_event(event: &str, payload: &Value) -> Option<AgentStatus> {
    match event {
        // Treat tool usage / prompt submit as "running"
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => Some(AgentStatus::Running),
        // Stop means the agent is no longer active
        "Stop" => Some(AgentStatus::Stopped),
        // Only permission prompt notification is actionable for UI status
        "Notification" => match extract_notification_type(payload) {
            Some("permission_prompt") => Some(AgentStatus::WaitingInput),
            _ => None,
        },
        _ => None,
    }
}

/// Process a Claude hook event payload and update the corresponding session status.
///
/// - If payload is empty or does not include a worktree path, this is a no-op.
/// - If there is no existing session for the worktree, this is a no-op.
pub fn process_claude_hook_event(event: &str, payload_json: &str) -> Result<()> {
    if payload_json.trim().is_empty() {
        return Ok(());
    }

    let payload: Value =
        serde_json::from_str(payload_json).map_err(|e| GwtError::ConfigParseError {
            reason: format!("Failed to parse hook payload JSON: {}", e),
        })?;

    let Some(worktree_path) = extract_worktree_path(&payload) else {
        return Ok(());
    };

    let Some(new_status) = status_for_event(event, &payload) else {
        return Ok(());
    };

    let Some(mut session) = Session::load_for_worktree(&worktree_path) else {
        // User preference: do not create sessions from hook events.
        return Ok(());
    };

    session.update_status(new_status);
    session.save(&Session::session_path(worktree_path.as_path()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::ffi::OsString;
    use std::path::Path;
    use std::sync::MutexGuard;
    use tempfile::TempDir;

    struct EnvVarGuard {
        key: &'static str,
        prev: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            let prev = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn prepare_session_dir() -> (MutexGuard<'static, ()>, TempDir, EnvVarGuard) {
        // Tests mutate global env vars; serialize to avoid flakiness.
        let lock = crate::config::HOME_LOCK.lock().unwrap();
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        let guard = EnvVarGuard::set("GWT_SESSIONS_DIR", &sessions_dir);
        (lock, tmp, guard)
    }

    fn create_session_for(worktree_path: &Path) {
        let session = Session::new(worktree_path, "test-branch");
        session.save(&Session::session_path(worktree_path)).unwrap();
    }

    #[test]
    fn hook_pre_tool_use_sets_running_when_session_exists() {
        let (_lock, tmp, _env) = prepare_session_dir();
        let worktree = tmp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        create_session_for(&worktree);

        let payload = json!({ "cwd": worktree.to_string_lossy(), "session_id": "test-123" });
        process_claude_hook_event("PreToolUse", &payload.to_string()).unwrap();

        let session = Session::load_for_worktree(&worktree).unwrap();
        assert_eq!(session.status, AgentStatus::Running);
    }

    #[test]
    fn hook_notification_permission_prompt_sets_waiting_input() {
        let (_lock, tmp, _env) = prepare_session_dir();
        let worktree = tmp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        create_session_for(&worktree);

        let payload = json!({
            "cwd": worktree.to_string_lossy(),
            "notification_type": "permission_prompt"
        });
        process_claude_hook_event("Notification", &payload.to_string()).unwrap();

        let session = Session::load_for_worktree(&worktree).unwrap();
        assert_eq!(session.status, AgentStatus::WaitingInput);
    }

    #[test]
    fn hook_stop_sets_stopped() {
        let (_lock, tmp, _env) = prepare_session_dir();
        let worktree = tmp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        create_session_for(&worktree);

        let payload = json!({ "cwd": worktree.to_string_lossy() });
        process_claude_hook_event("Stop", &payload.to_string()).unwrap();

        let session = Session::load_for_worktree(&worktree).unwrap();
        assert_eq!(session.status, AgentStatus::Stopped);
    }

    #[test]
    fn hook_noop_when_session_missing() {
        let (_lock, tmp, _env) = prepare_session_dir();
        let worktree = tmp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let payload = json!({ "cwd": worktree.to_string_lossy() });
        process_claude_hook_event("PreToolUse", &payload.to_string()).unwrap();

        assert!(Session::load_for_worktree(&worktree).is_none());
    }
}
