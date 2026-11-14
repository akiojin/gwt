use crate::{
    daemon_runtime::{RuntimeHookEvent, RuntimeHookEventKind},
    persistence::WindowState,
    preset::WindowPreset,
};
use gwt_terminal::PaneStatus;

/// Provider family whose current rendered screen can author an exact-window
/// human-approval wait. Unknown providers deliberately remain unclassified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalPromptProvider {
    ClaudeCode,
    Codex,
    Unsupported,
}

/// Return a privacy-safe identity only for a complete approval prompt at the
/// bottom of the current vt100 screen. Provider-specific title, choices, and
/// footer are all required; quoted transcript blocks are not live prompts.
pub fn approval_prompt_fingerprint(provider: ApprovalPromptProvider, screen: &str) -> Option<u64> {
    if provider == ApprovalPromptProvider::Unsupported {
        return None;
    }

    let normalized = normalize_approval_screen(screen);
    let (titles, accepts, reject, footer): (&[&str], &[&str], &str, &str) = match provider {
        ApprovalPromptProvider::Codex => (
            &[
                "wouldyouliketorunthefollowingcommand?",
                "wouldyouliketograntthesepermissions?",
                "wouldyouliketomakethefollowingedits?",
                "needsyourapproval.",
            ],
            &["1.yes,proceed", "yes,proceed"],
            "no,andtellcodexwhattododifferently",
            "pressentertoconfirmoresctocancel",
        ),
        ApprovalPromptProvider::ClaudeCode => (
            &[
                "doyouwanttoproceed?",
                "doyouwanttomakethiseditto",
                "doyouwanttoallowclaudetofetchthiscontent?",
            ],
            &[
                "1.yes",
                "yes,duringthissession",
                "yes,allow",
                "yes,anddon'taskagain",
                "yes,anddon’taskagain",
            ],
            "no,andtellclaudewhattododifferently",
            "entertoconfirm·esctocancel",
        ),
        ApprovalPromptProvider::Unsupported => return None,
    };

    if !normalized.ends_with(footer) {
        return None;
    }
    let (title, title_start, title_end) = titles
        .iter()
        .filter_map(|title| {
            normalized
                .rfind(title)
                .map(|start| (*title, start, start + title.len()))
        })
        .max_by_key(|(_, start, _)| *start)?;
    if latest_title_is_quoted(screen, title) {
        return None;
    }
    let accept_end = accepts
        .iter()
        .filter_map(|accept| {
            normalized[title_end..]
                .find(accept)
                .map(|start| title_end + start + accept.len())
        })
        .min()?;
    let reject_end = normalized[accept_end..]
        .find(reject)
        .map(|start| accept_end + start + reject.len())?;
    let footer_start = normalized[reject_end..]
        .find(footer)
        .map(|start| reject_end + start)?;
    if footer_start + footer.len() != normalized.len()
        || !has_selected_choice_in_latest_block(screen, title, &normalized[title_end..footer_start])
    {
        return None;
    }

    Some(fnv1a64(
        match provider {
            ApprovalPromptProvider::Codex => b"codex:".as_slice(),
            ApprovalPromptProvider::ClaudeCode => b"claude-code:".as_slice(),
            ApprovalPromptProvider::Unsupported => return None,
        },
        &normalized.as_bytes()[title_start..],
    ))
}

/// Whether an incomplete redraw still carries distinctive approval UI. This
/// is used only to retain an already-authorized latch, never to create one.
pub fn has_approval_prompt_evidence(provider: ApprovalPromptProvider, screen: &str) -> bool {
    let normalized = normalize_approval_screen(screen);
    let evidence: &[&str] = match provider {
        ApprovalPromptProvider::Codex => &[
            "wouldyouliketorunthefollowingcommand?",
            "wouldyouliketograntthesepermissions?",
            "wouldyouliketomakethefollowingedits?",
            "needsyourapproval.",
            "no,andtellcodexwhattododifferently",
            "pressentertoconfirmoresctocancel",
        ],
        ApprovalPromptProvider::ClaudeCode => &[
            "doyouwanttoproceed?",
            "doyouwanttomakethiseditto",
            "doyouwanttoallowclaudetofetchthiscontent?",
            "no,andtellclaudewhattododifferently",
            "entertoconfirm·esctocancel",
        ],
        ApprovalPromptProvider::Unsupported => return false,
    };
    let prompt_chrome_reaches_bottom = evidence.iter().any(|phrase| normalized.ends_with(phrase))
        || screen
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .and_then(selected_choice_body)
            .is_some();
    prompt_chrome_reaches_bottom && evidence.iter().any(|phrase| normalized.contains(phrase))
}

fn normalize_approval_screen(screen: &str) -> String {
    screen
        .lines()
        .map(|line| selected_choice_body(line).unwrap_or(line))
        .flat_map(str::chars)
        .filter(|character| {
            !character.is_whitespace() && !matches!(character, '›' | '❯' | '▸' | '▶')
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn latest_title_is_quoted(screen: &str, normalized_title: &str) -> bool {
    screen
        .lines()
        .rev()
        .find(|line| normalize_approval_screen(line).contains(normalized_title))
        .is_some_and(|line| line.trim_start().starts_with('>'))
}

fn has_selected_choice_in_latest_block(
    screen: &str,
    normalized_title: &str,
    normalized_block: &str,
) -> bool {
    let lines = screen.lines().collect::<Vec<_>>();
    let title_line = (0..lines.len()).rev().find(|start| {
        normalize_approval_screen(&lines[*start..].join("\n")).contains(normalized_title)
    });
    let Some(title_line) = title_line else {
        return false;
    };
    lines[title_line..].iter().any(|line| {
        let Some(rest) = selected_choice_body(line) else {
            return false;
        };
        let normalized_choice = normalize_approval_screen(rest);
        !normalized_choice.is_empty() && normalized_block.contains(&normalized_choice)
    })
}

fn selected_choice_body(line: &str) -> Option<&str> {
    let rest = line
        .trim_start()
        .strip_prefix(['›', '❯', '▸', '▶', '>'])?
        .trim_start();
    matches!(rest.as_bytes(), [b'0'..=b'9', b'.', ..]).then_some(rest)
}

fn fnv1a64(prefix: &[u8], value: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in prefix.iter().chain(value) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

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
        approval_prompt_fingerprint, compose_window_state,
        compose_window_state_with_active_session, has_approval_prompt_evidence,
        runtime_hook_window_state, window_state_from_pane_status, ApprovalPromptProvider,
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
            continuation_readiness_nonce: None,
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

    #[test]
    fn t496_approval_detector_accepts_only_complete_provider_specific_bottom_prompt() {
        let codex = "Would you like to run the following command?\n\n  cargo test\n\n\
            › 1. Yes, proceed\n  2. No, and tell Codex what to do differently\n\n\
            Press enter to confirm or esc to cancel";
        let claude = "Bash command\n\n  cargo test\n\nDo you want to proceed?\n\
            ❯ 1. Yes\n  2. No, and tell Claude what to do differently\n\n\
            Enter to confirm · Esc to cancel";

        assert!(approval_prompt_fingerprint(ApprovalPromptProvider::Codex, codex).is_some());
        assert!(approval_prompt_fingerprint(ApprovalPromptProvider::ClaudeCode, claude).is_some());
        assert!(approval_prompt_fingerprint(ApprovalPromptProvider::ClaudeCode, codex).is_none());
        assert!(approval_prompt_fingerprint(ApprovalPromptProvider::Unsupported, codex).is_none());
    }

    #[test]
    fn t496_approval_detector_rejects_partial_redraw_and_quoted_history() {
        let partial = "Would you like to run the following command?\n› 1. Yes, proceed";
        let historical = "Would you like to run the following command?\n\
            › 1. Yes, proceed\n  2. No, and tell Codex what to do differently\n\
            Press enter to confirm or esc to cancel\ncommand completed";
        let quoted = "> Would you like to run the following command?\n\
            > 1. Yes, proceed\n> 2. No, and tell Codex what to do differently\n\
            > Press enter to confirm or esc to cancel";

        assert!(approval_prompt_fingerprint(ApprovalPromptProvider::Codex, partial).is_none());
        assert!(approval_prompt_fingerprint(ApprovalPromptProvider::Codex, historical).is_none());
        assert!(approval_prompt_fingerprint(ApprovalPromptProvider::Codex, quoted).is_none());
        assert!(has_approval_prompt_evidence(
            ApprovalPromptProvider::Codex,
            partial
        ));
        assert!(!has_approval_prompt_evidence(
            ApprovalPromptProvider::Codex,
            historical
        ));
    }
}
