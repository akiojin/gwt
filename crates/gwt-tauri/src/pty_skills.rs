//! PTY communication skill API
//!
//! Thin wrappers around existing terminal functions providing a clean skill interface
//! for Project Mode PTY operations.
//!
//! # Equivalence to agent_tools.rs terminal tools
//!
//! This module provides the same PTY functionality as the built-in tool definitions
//! in `agent_tools.rs`, but with a cleaner Rust API that returns structured
//! `PtySkillResult` values instead of raw `Result<String, String>`.
//!
//! | pty_skills function | Replaces agent_tools tool constant        |
//! |---------------------|-------------------------------------------|
//! | `send_to_pane()`    | `TOOL_SEND_KEYS_TO_PANE` (send_keys_to_pane) |
//! | `broadcast()`       | `TOOL_SEND_KEYS_BROADCAST` (send_keys_broadcast) |
//! | `capture_output()`  | `TOOL_CAPTURE_SCROLLBACK_TAIL` (capture_scrollback_tail) |
//! | `list_panes()`      | (no equivalent in agent_tools — new addition) |
//!
//! Both modules delegate to the same underlying functions in
//! `crate::commands::terminal`. The `agent_tools` module is retained for
//! backward compatibility with the existing LLM tool-call dispatch path.

// Functions will be called from ReAct loop integration (Phase 9-12).
#![allow(dead_code)]

use crate::commands::terminal::{
    capture_scrollback_tail_from_state, send_keys_broadcast_from_state,
    send_keys_to_pane_from_state,
};
use crate::state::AppState;
use gwt_core::terminal::pane::PaneStatus;

/// Result of a PTY skill operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PtySkillResult {
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
}

/// Information about a running pane.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PaneInfo {
    pub pane_id: String,
    pub agent_name: Option<String>,
    pub branch_name: Option<String>,
    pub status: String,
}

/// Send input to a specific pane.
pub fn send_to_pane(state: &AppState, pane_id: &str, text: &str) -> PtySkillResult {
    match send_keys_to_pane_from_state(state, pane_id, text) {
        Ok(()) => PtySkillResult {
            success: true,
            output: None,
            error: None,
        },
        Err(e) => PtySkillResult {
            success: false,
            output: None,
            error: Some(e),
        },
    }
}

/// Broadcast input to all running panes.
pub fn broadcast(state: &AppState, text: &str) -> PtySkillResult {
    match send_keys_broadcast_from_state(state, text) {
        Ok(sent) => PtySkillResult {
            success: true,
            output: Some(sent.to_string()),
            error: None,
        },
        Err(e) => PtySkillResult {
            success: false,
            output: None,
            error: Some(e),
        },
    }
}

/// Capture recent output from a pane.
pub fn capture_output(state: &AppState, pane_id: &str, max_bytes: Option<usize>) -> PtySkillResult {
    let limit = max_bytes.unwrap_or(0);
    match capture_scrollback_tail_from_state(state, pane_id, limit) {
        Ok(text) => PtySkillResult {
            success: true,
            output: Some(text),
            error: None,
        },
        Err(e) => PtySkillResult {
            success: false,
            output: None,
            error: Some(e),
        },
    }
}

/// List all panes with their info.
pub fn list_panes(state: &AppState) -> Vec<PaneInfo> {
    let manager = match state.pane_manager.lock() {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    manager
        .panes()
        .iter()
        .map(|pane| {
            let status = match pane.status() {
                PaneStatus::Running => "running".to_string(),
                PaneStatus::Completed(code) => format!("completed({})", code),
                PaneStatus::Error(msg) => format!("error: {}", msg),
            };
            PaneInfo {
                pane_id: pane.pane_id().to_string(),
                agent_name: Some(pane.agent_name().to_string()),
                branch_name: Some(pane.branch_name().to_string()),
                status,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::TestEnvGuard;
    use crate::commands::ENV_LOCK;
    use gwt_core::terminal::pane::{PaneConfig, TerminalPane};
    use gwt_core::terminal::AgentColor;

    #[test]
    fn pty_skill_result_serialization_roundtrip() {
        let result = PtySkillResult {
            success: true,
            output: Some("hello".to_string()),
            error: None,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let deserialized: PtySkillResult = serde_json::from_str(&json).expect("deserialize");
        assert!(deserialized.success);
        assert_eq!(deserialized.output.as_deref(), Some("hello"));
        assert!(deserialized.error.is_none());
    }

    #[test]
    fn pty_skill_result_error_serialization_roundtrip() {
        let result = PtySkillResult {
            success: false,
            output: None,
            error: Some("pane not found".to_string()),
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let deserialized: PtySkillResult = serde_json::from_str(&json).expect("deserialize");
        assert!(!deserialized.success);
        assert!(deserialized.output.is_none());
        assert_eq!(deserialized.error.as_deref(), Some("pane not found"));
    }

    #[test]
    fn pane_info_serialization_roundtrip() {
        let info = PaneInfo {
            pane_id: "pane-1".to_string(),
            agent_name: Some("agent-a".to_string()),
            branch_name: Some("feature/test".to_string()),
            status: "running".to_string(),
        };
        let json = serde_json::to_string(&info).expect("serialize");
        let deserialized: PaneInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.pane_id, "pane-1");
        assert_eq!(deserialized.agent_name.as_deref(), Some("agent-a"));
        assert_eq!(deserialized.branch_name.as_deref(), Some("feature/test"));
        assert_eq!(deserialized.status, "running");
    }

    #[test]
    fn pane_info_with_none_fields_serialization_roundtrip() {
        let info = PaneInfo {
            pane_id: "pane-2".to_string(),
            agent_name: None,
            branch_name: None,
            status: "completed(0)".to_string(),
        };
        let json = serde_json::to_string(&info).expect("serialize");
        let deserialized: PaneInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.pane_id, "pane-2");
        assert!(deserialized.agent_name.is_none());
        assert!(deserialized.branch_name.is_none());
    }

    #[test]
    fn send_to_pane_invalid_pane_returns_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let state = AppState::new();
        let result = send_to_pane(&state, "nonexistent-pane", "hello");
        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.output.is_none());
    }

    #[test]
    fn capture_output_invalid_pane_returns_error() {
        let _lock = ENV_LOCK.lock().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let state = AppState::new();
        let result = capture_output(&state, "nonexistent-pane", None);
        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.output.is_none());
    }

    #[test]
    fn list_panes_empty_when_no_panes() {
        let state = AppState::new();
        let panes = list_panes(&state);
        assert!(panes.is_empty());
    }

    #[test]
    fn list_panes_returns_pane_info() {
        let _lock = ENV_LOCK.lock().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let state = AppState::new();
        let pane = TerminalPane::new(PaneConfig {
            pane_id: "pane-list-test".to_string(),
            command: "/bin/cat".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "test-branch".to_string(),
            agent_name: "test-agent".to_string(),
            agent_color: AgentColor::Green,
            rows: 24,
            cols: 80,
            env_vars: Default::default(),
            terminal_shell: None,
            interactive: false,
        })
        .expect("failed to create test pane");

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            mgr.add_pane(pane).expect("failed to add test pane");
        }

        let panes = list_panes(&state);
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].pane_id, "pane-list-test");
        assert_eq!(panes[0].agent_name.as_deref(), Some("test-agent"));
        assert_eq!(panes[0].branch_name.as_deref(), Some("test-branch"));
        assert_eq!(panes[0].status, "running");
    }

    #[test]
    fn broadcast_empty_state_returns_zero_sent() {
        let state = AppState::new();
        let result = broadcast(&state, "hello");
        assert!(result.success);
        assert_eq!(result.output.as_deref(), Some("0"));
    }

    #[test]
    fn send_to_pane_success_with_running_pane() {
        let _lock = ENV_LOCK.lock().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let state = AppState::new();
        let pane = TerminalPane::new(PaneConfig {
            pane_id: "pane-send-test".to_string(),
            command: "/bin/cat".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            branch_name: "test-branch".to_string(),
            agent_name: "test-agent".to_string(),
            agent_color: AgentColor::Green,
            rows: 24,
            cols: 80,
            env_vars: Default::default(),
            terminal_shell: None,
            interactive: false,
        })
        .expect("failed to create test pane");

        {
            let mut mgr = state.pane_manager.lock().unwrap();
            mgr.add_pane(pane).expect("failed to add test pane");
        }

        let result = send_to_pane(&state, "pane-send-test", "hello\n");
        assert!(result.success);
        assert!(result.error.is_none());
    }
}
