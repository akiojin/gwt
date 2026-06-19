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
    compose_window_state_with_active_session(pty_state, preset, hook_state, false)
}

pub fn compose_window_state_with_active_session(
    pty_state: WindowState,
    preset: WindowPreset,
    hook_state: Option<WindowState>,
    has_active_agent_session: bool,
) -> WindowState {
    if pty_state == WindowState::Error && has_active_agent_session && uses_agent_hook_state(preset)
    {
        if let Some(hook_state) = hook_state.filter(|state| is_live_agent_hook_state(*state)) {
            return hook_state;
        }
    }
    if matches!(pty_state, WindowState::Stopped | WindowState::Error) {
        return pty_state;
    }
    if uses_agent_hook_state(preset) {
        return hook_state.unwrap_or(if has_active_agent_session {
            WindowState::Idle
        } else {
            WindowState::Starting
        });
    }
    pty_state
}

pub fn is_live_agent_hook_state(state: WindowState) -> bool {
    matches!(
        state,
        WindowState::Running | WindowState::Waiting | WindowState::Idle
    )
}

pub fn runtime_hook_window_state(event: &RuntimeHookEvent) -> Option<WindowState> {
    if event.kind != RuntimeHookEventKind::RuntimeState {
        return None;
    }
    let source_event = event.source_event.as_deref();
    if source_event == Some("SessionStart") {
        return Some(WindowState::Idle);
    }
    let status_state = event.status.as_deref().and_then(parse_runtime_status);
    if source_event == Some("Stop") && status_state == Some(WindowState::Waiting) {
        return Some(WindowState::Idle);
    }
    status_state.or_else(|| source_event.and_then(window_state_for_hook_event))
}

pub fn window_state_from_pane_status(status: &PaneStatus) -> WindowState {
    match status {
        PaneStatus::Running => WindowState::Running,
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
        "SessionStart" | "Stop" => Some(WindowState::Idle),
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => Some(WindowState::Running),
        _ => None,
    }
}

fn parse_runtime_status(status: &str) -> Option<WindowState> {
    match status.trim().to_ascii_lowercase().as_str() {
        "running" | "ready" => Some(WindowState::Running),
        "starting" | "notstarted" | "not_started" | "not-started" | "not started" => {
            Some(WindowState::Starting)
        }
        "idle" => Some(WindowState::Idle),
        "waiting" | "waitinginput" | "waiting_input" => Some(WindowState::Waiting),
        "stopped" | "exited" => Some(WindowState::Stopped),
        "error" => Some(WindowState::Error),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compose_window_state, compose_window_state_with_active_session, runtime_hook_window_state,
        window_state_from_pane_status,
    };
    use crate::{
        daemon_runtime::{RuntimeHookEvent, RuntimeHookEventKind},
        persistence::WindowState,
        preset::WindowPreset,
    };
    use gwt_terminal::PaneStatus;

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
            WindowState::Starting
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
    fn runtime_hook_window_state_maps_runtime_events_to_running_and_idle() {
        assert_eq!(
            runtime_hook_window_state(&runtime_event(Some("Running"), Some("PreToolUse"))),
            Some(WindowState::Running)
        );
        assert_eq!(
            serde_json::to_string(
                &runtime_hook_window_state(&runtime_event(Some("Idle"), Some("Stop"))).unwrap()
            )
            .unwrap(),
            "\"idle\""
        );
        assert_eq!(
            serde_json::to_string(
                &runtime_hook_window_state(&runtime_event(None, Some("SessionStart"))).unwrap()
            )
            .unwrap(),
            "\"idle\""
        );
        assert_eq!(
            serde_json::to_string(
                &runtime_hook_window_state(&runtime_event(None, Some("Stop"))).unwrap()
            )
            .unwrap(),
            "\"idle\""
        );
    }

    #[test]
    fn compose_window_state_defaults_live_agent_without_hook_state_to_starting() {
        let composed = compose_window_state(WindowState::Running, WindowPreset::Agent, None);

        assert_eq!(composed, WindowState::Starting);
        assert_eq!(serde_json::to_string(&composed).unwrap(), "\"starting\"");
    }

    #[test]
    fn compose_window_state_defaults_active_agent_without_hook_state_to_idle() {
        let composed = compose_window_state_with_active_session(
            WindowState::Running,
            WindowPreset::Agent,
            None,
            true,
        );

        assert_eq!(composed, WindowState::Idle);
        assert_eq!(serde_json::to_string(&composed).unwrap(), "\"idle\"");
    }

    #[test]
    fn compose_window_state_recovers_active_agent_from_stale_pty_error_state() {
        assert_eq!(
            compose_window_state_with_active_session(
                WindowState::Error,
                WindowPreset::Codex,
                Some(WindowState::Running),
                true,
            ),
            WindowState::Running
        );
    }

    #[test]
    fn compose_window_state_keeps_pty_stopped_for_active_agent_recovery() {
        assert_eq!(
            compose_window_state_with_active_session(
                WindowState::Stopped,
                WindowPreset::Claude,
                Some(WindowState::Waiting),
                true,
            ),
            WindowState::Stopped
        );
        assert_eq!(
            compose_window_state_with_active_session(
                WindowState::Stopped,
                WindowPreset::Agent,
                Some(WindowState::Idle),
                true,
            ),
            WindowState::Stopped
        );
    }

    #[test]
    fn compose_window_state_keeps_pty_terminal_state_without_active_agent_recovery() {
        assert_eq!(
            compose_window_state_with_active_session(
                WindowState::Error,
                WindowPreset::Shell,
                Some(WindowState::Running),
                true,
            ),
            WindowState::Error
        );
        assert_eq!(
            compose_window_state_with_active_session(
                WindowState::Error,
                WindowPreset::Codex,
                Some(WindowState::Running),
                false,
            ),
            WindowState::Error
        );
        assert_eq!(
            compose_window_state_with_active_session(
                WindowState::Error,
                WindowPreset::Codex,
                Some(WindowState::Error),
                true,
            ),
            WindowState::Error
        );
    }

    #[test]
    fn runtime_hook_window_state_ignores_non_runtime_events() {
        let mut event = runtime_event(Some("Running"), Some("PreToolUse"));
        event.kind = RuntimeHookEventKind::Forward;
        assert_eq!(runtime_hook_window_state(&event), None);
    }

    #[test]
    fn pane_status_running_maps_to_running_window_state() {
        assert_eq!(
            window_state_from_pane_status(&PaneStatus::Running),
            WindowState::Running
        );
        assert_eq!(
            window_state_from_pane_status(&PaneStatus::Completed(0)),
            WindowState::Stopped
        );
        assert_eq!(
            window_state_from_pane_status(&PaneStatus::Completed(1)),
            WindowState::Error
        );
        assert_eq!(
            window_state_from_pane_status(&PaneStatus::Error("boom".to_string())),
            WindowState::Error
        );
    }
}
