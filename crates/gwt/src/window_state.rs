use crate::{
    daemon_runtime::{RuntimeHookEvent, RuntimeHookEventKind},
    persistence::WindowState,
    preset::WindowPreset,
};
use gwt_terminal::PaneStatus;

pub fn compose_window_state(
    pty_state: WindowState,
    preset: WindowPreset,
    hook_state: Option<WindowState>,
) -> WindowState {
    if matches!(pty_state, WindowState::Stopped | WindowState::Error) {
        return pty_state;
    }
    if uses_agent_hook_state(preset) {
        return hook_state.unwrap_or(WindowState::Running);
    }
    pty_state
}

pub fn runtime_hook_window_state(event: &RuntimeHookEvent) -> Option<WindowState> {
    if event.kind != RuntimeHookEventKind::RuntimeState {
        return None;
    }
    event
        .status
        .as_deref()
        .and_then(parse_runtime_status)
        .or_else(|| {
            event
                .source_event
                .as_deref()
                .and_then(window_state_for_hook_event)
        })
}

pub fn window_state_from_pane_status(status: &PaneStatus) -> WindowState {
    match status {
        PaneStatus::Running => WindowState::Stopped,
        PaneStatus::Completed(0) => WindowState::Stopped,
        PaneStatus::Completed(_) | PaneStatus::Error(_) => WindowState::Error,
    }
}

pub fn uses_agent_hook_state(preset: WindowPreset) -> bool {
    matches!(
        preset,
        WindowPreset::Agent | WindowPreset::Claude | WindowPreset::Codex
    )
}

pub fn window_state_for_hook_event(event: &str) -> Option<WindowState> {
    match event {
        "SessionStart" | "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => {
            Some(WindowState::Running)
        }
        "Stop" => Some(WindowState::Waiting),
        _ => None,
    }
}

fn parse_runtime_status(status: &str) -> Option<WindowState> {
    match status.trim().to_ascii_lowercase().as_str() {
        "running" | "starting" | "ready" => Some(WindowState::Running),
        "waiting" | "waitinginput" | "waiting_input" => Some(WindowState::Waiting),
        "stopped" | "exited" => Some(WindowState::Stopped),
        "error" => Some(WindowState::Error),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{compose_window_state, runtime_hook_window_state};
    use crate::{
        daemon_runtime::{RuntimeHookEvent, RuntimeHookEventKind},
        persistence::WindowState,
        preset::WindowPreset,
    };

    fn runtime_event(status: Option<&str>, source_event: Option<&str>) -> RuntimeHookEvent {
        RuntimeHookEvent {
            kind: RuntimeHookEventKind::RuntimeState,
            source_event: source_event.map(str::to_string),
            gwt_session_id: Some("session-1".to_string()),
            agent_session_id: Some("agent-1".to_string()),
            project_root: Some("E:/gwt/test".to_string()),
            branch: Some("feature/runtime".to_string()),
            status: status.map(str::to_string),
            tool_name: None,
            message: None,
            occurred_at: "2026-04-22T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn compose_window_state_prefers_hook_state_for_live_agent_windows() {
        assert_eq!(
            compose_window_state(
                WindowState::Running,
                WindowPreset::Agent,
                Some(WindowState::Waiting),
            ),
            WindowState::Waiting
        );
        assert_eq!(
            compose_window_state(
                WindowState::Running,
                WindowPreset::Claude,
                Some(WindowState::Running),
            ),
            WindowState::Running
        );
    }

    #[test]
    fn compose_window_state_follows_pty_and_preset_rules() {
        assert_eq!(
            compose_window_state(
                WindowState::Stopped,
                WindowPreset::Agent,
                Some(WindowState::Waiting),
            ),
            WindowState::Stopped
        );
        assert_eq!(
            compose_window_state(WindowState::Running, WindowPreset::Agent, None),
            WindowState::Running
        );
        assert_eq!(
            compose_window_state(
                WindowState::Running,
                WindowPreset::Shell,
                Some(WindowState::Waiting),
            ),
            WindowState::Running
        );
    }

    #[test]
    fn runtime_hook_window_state_maps_runtime_events_to_running_and_waiting() {
        assert_eq!(
            runtime_hook_window_state(&runtime_event(Some("Running"), Some("PreToolUse"))),
            Some(WindowState::Running)
        );
        assert_eq!(
            runtime_hook_window_state(&runtime_event(Some("Waiting"), Some("Stop"))),
            Some(WindowState::Waiting)
        );
        assert_eq!(
            runtime_hook_window_state(&runtime_event(None, Some("SessionStart"))),
            Some(WindowState::Running)
        );
        assert_eq!(
            runtime_hook_window_state(&runtime_event(None, Some("Stop"))),
            Some(WindowState::Waiting)
        );
    }

    #[test]
    fn runtime_hook_window_state_ignores_non_runtime_events() {
        let mut event = runtime_event(Some("Running"), Some("PreToolUse"));
        event.kind = RuntimeHookEventKind::Forward;
        assert_eq!(runtime_hook_window_state(&event), None);
    }
}
