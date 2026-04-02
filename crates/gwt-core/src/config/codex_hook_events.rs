//! Codex Hook event processing (SPEC-1438 FR-HOOK-001..004)
//!
//! Processes hook payloads from Codex CLI and updates gwt session status.
//! Codex supports 5 events: SessionStart, PreToolUse, PostToolUse,
//! UserPromptSubmit, Stop.

use std::path::PathBuf;

use serde_json::Value;

use super::session::{AgentStatus, Session};
use crate::error::{GwtError, Result};

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

fn status_for_event(event: &str) -> Option<AgentStatus> {
    match event {
        "SessionStart" | "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => {
            Some(AgentStatus::Running)
        }
        "Stop" => Some(AgentStatus::Stopped),
        _ => None,
    }
}

/// Process a Codex hook event payload and update the corresponding session status.
///
/// - If payload is empty or does not include a worktree path, this is a no-op.
/// - If there is no existing session for the worktree, this is a no-op.
pub fn process_codex_hook_event(event: &str, payload_json: &str) -> Result<()> {
    if payload_json.trim().is_empty() {
        return Ok(());
    }

    let payload: Value =
        serde_json::from_str(payload_json).map_err(|e| GwtError::ConfigParseError {
            reason: format!("Failed to parse Codex hook payload JSON: {}", e),
        })?;

    let Some(worktree_path) = extract_worktree_path(&payload) else {
        return Ok(());
    };

    let Some(new_status) = status_for_event(event) else {
        return Ok(());
    };

    let Some(mut session) = Session::load_for_worktree(&worktree_path) else {
        return Ok(());
    };

    session.update_status(new_status);
    session.save(&Session::session_path(worktree_path.as_path()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::Path, sync::MutexGuard};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

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
    fn codex_hook_session_start_sets_running() {
        let (_lock, tmp, _env) = prepare_session_dir();
        let worktree = tmp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        create_session_for(&worktree);

        let payload = json!({ "cwd": worktree.to_string_lossy() });
        process_codex_hook_event("SessionStart", &payload.to_string()).unwrap();

        let session = Session::load_for_worktree(&worktree).unwrap();
        assert_eq!(session.status, AgentStatus::Running);
    }

    #[test]
    fn codex_hook_pre_tool_use_sets_running() {
        let (_lock, tmp, _env) = prepare_session_dir();
        let worktree = tmp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        create_session_for(&worktree);

        let payload = json!({ "cwd": worktree.to_string_lossy() });
        process_codex_hook_event("PreToolUse", &payload.to_string()).unwrap();

        let session = Session::load_for_worktree(&worktree).unwrap();
        assert_eq!(session.status, AgentStatus::Running);
    }

    #[test]
    fn codex_hook_stop_sets_stopped() {
        let (_lock, tmp, _env) = prepare_session_dir();
        let worktree = tmp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        create_session_for(&worktree);

        let payload = json!({ "cwd": worktree.to_string_lossy() });
        process_codex_hook_event("Stop", &payload.to_string()).unwrap();

        let session = Session::load_for_worktree(&worktree).unwrap();
        assert_eq!(session.status, AgentStatus::Stopped);
    }

    #[test]
    fn codex_hook_unknown_event_is_noop() {
        let (_lock, tmp, _env) = prepare_session_dir();
        let worktree = tmp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        create_session_for(&worktree);

        let payload = json!({ "cwd": worktree.to_string_lossy() });
        // Notification is not supported by Codex
        process_codex_hook_event("Notification", &payload.to_string()).unwrap();

        let session = Session::load_for_worktree(&worktree).unwrap();
        assert_eq!(session.status, AgentStatus::Unknown);
    }

    #[test]
    fn codex_hook_noop_when_session_missing() {
        let (_lock, tmp, _env) = prepare_session_dir();
        let worktree = tmp.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        let payload = json!({ "cwd": worktree.to_string_lossy() });
        process_codex_hook_event("PreToolUse", &payload.to_string()).unwrap();

        assert!(Session::load_for_worktree(&worktree).is_none());
    }
}
