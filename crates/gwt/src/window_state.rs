use crate::{
    daemon_runtime::{RuntimeHookEvent, RuntimeHookEventKind},
    persistence::WindowState,
    preset::WindowPreset,
};
use gwt_terminal::PaneStatus;

/// Provider family whose current rendered screen can be classified for a
/// human tool-approval prompt. Unsupported providers deliberately fail open:
/// a false positive is more disruptive than leaving an unknown prompt Idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalPromptProvider {
    ClaudeCode,
    Codex,
    Unsupported,
}

/// Return a privacy-safe identity for a fully rendered provider approval
/// prompt. The classifier is anchored to the bottom of the current vt100
/// screen and requires the provider's ordered title, affirmative choice,
/// reject choice, and footer. Whitespace is removed so terminal soft wrapping
/// at different widths does not change the result. Selection glyphs are also
/// excluded from the hash so arrow-key navigation remains the same prompt.
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
    if footer_start + footer.len() != normalized.len() {
        return None;
    }
    if !has_selected_choice_in_latest_block(screen, title, &normalized[title_end..footer_start]) {
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

/// Boolean compatibility wrapper for callers that do not need prompt
/// identity.
pub fn detect_approval_prompt(provider: ApprovalPromptProvider, screen: &str) -> bool {
    approval_prompt_fingerprint(provider, screen).is_some()
}

/// Whether an incomplete screen still carries provider-specific approval UI
/// evidence. This deliberately recognizes only distinctive title, reject, or
/// footer phrases; ordinary command output must not keep a stale wait latched.
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
    evidence.iter().any(|phrase| normalized.ends_with(phrase))
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
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix(['›', '❯', '▸', '▶', '>'])?;
    let rest = rest.trim_start();
    if matches!(rest.as_bytes(), [b'0'..=b'9', b'.', ..]) {
        Some(rest)
    } else {
        None
    }
}

fn fnv1a64(prefix: &[u8], value: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in prefix.iter().chain(value) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Whether one byte-exact terminal input resolves the currently selected
/// approval choice. Arrow/function-key escape sequences and ordinary typing
/// must not clear the wait overlay.
pub fn is_approval_resolution_input(data: &str) -> bool {
    data == "\u{1b}" || data.ends_with('\r') || data.ends_with('\n')
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

/// Compose the transient approval overlay without overwriting the underlying
/// hook state. Terminal lifecycle evidence remains authoritative; otherwise a
/// supported live Agent prompt projects through the existing Waiting state.
pub fn compose_window_state_with_approval_wait(
    pty_state: WindowState,
    preset: WindowPreset,
    hook_state: Option<WindowState>,
    has_active_agent_session: bool,
    approval_waiting: bool,
) -> WindowState {
    if approval_waiting && matches!(pty_state, WindowState::Stopped | WindowState::Error) {
        return pty_state;
    }
    let composed = compose_window_state_with_active_session(
        pty_state,
        preset,
        hook_state,
        has_active_agent_session,
    );
    if approval_waiting
        && uses_agent_hook_state(preset)
        && !matches!(composed, WindowState::Stopped | WindowState::Error)
    {
        WindowState::Waiting
    } else {
        composed
    }
}

/// Issue #3616: present a quota-blocked agent pane as waiting, not finished.
///
/// The PTY really did exit, so the underlying state is honest — but `Stopped`
/// renders as `DONE`, which claims the work completed, and `Error` claims the
/// agent broke. Neither happened: the account ran out and the conversation is
/// intact. `Waiting` is the existing state for "this pane needs something from
/// outside before it can continue", which is exactly the situation.
///
/// Deliberately narrow: only the two terminal states are overridden, so a live
/// pane that merely rendered the sentence keeps whatever state it had.
pub fn apply_provider_quota_block(composed: WindowState, quota_blocked: bool) -> WindowState {
    if quota_blocked && matches!(composed, WindowState::Stopped | WindowState::Error) {
        WindowState::Waiting
    } else {
        composed
    }
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
        compose_window_state_with_active_session, compose_window_state_with_approval_wait,
        detect_approval_prompt, has_approval_prompt_evidence, is_approval_resolution_input,
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
    fn approval_prompt_classifier_requires_complete_codex_prompt_structure() {
        let prompt = r#"
Would you like to run the following command?

  cargo test -p gwt window_state

› 1. Yes, proceed
  2. Yes, and don't ask again for commands that start with `cargo test`
  3. No, and tell Codex what to do differently

Press enter to confirm or esc to cancel
"#;

        assert!(detect_approval_prompt(
            ApprovalPromptProvider::Codex,
            prompt
        ));
        assert!(!detect_approval_prompt(
            ApprovalPromptProvider::Codex,
            "The docs say: Would you like to run the following command? Yes, proceed."
        ));
        assert!(!detect_approval_prompt(
            ApprovalPromptProvider::Codex,
            "Would you like to run the following command?\n› 1. Yes, proceed"
        ));
    }

    #[test]
    fn approval_prompt_classifier_requires_complete_claude_prompt_structure() {
        let prompt = r#"
Bash command

  cargo test -p gwt window_state

Do you want to proceed?
❯ 1. Yes
  2. Yes, and don't ask again for cargo test commands in this project
  3. No, and tell Claude what to do differently

Enter to confirm · Esc to cancel
"#;

        assert!(detect_approval_prompt(
            ApprovalPromptProvider::ClaudeCode,
            prompt
        ));
        assert!(!detect_approval_prompt(
            ApprovalPromptProvider::ClaudeCode,
            "Do you want to proceed?"
        ));
        assert!(!detect_approval_prompt(
            ApprovalPromptProvider::ClaudeCode,
            "Do you want to proceed?\nYes\nNo"
        ));
    }

    #[test]
    fn approval_prompt_classifier_does_not_cross_provider_boundaries() {
        let codex_prompt = r#"
Would you like to make the following edits?
› 1. Yes, proceed
  2. No, and tell Codex what to do differently
Press enter to confirm or esc to cancel
"#;

        assert!(!detect_approval_prompt(
            ApprovalPromptProvider::ClaudeCode,
            codex_prompt
        ));
        assert!(!detect_approval_prompt(
            ApprovalPromptProvider::Unsupported,
            codex_prompt
        ));
    }

    #[test]
    fn approval_prompt_classifier_is_bottom_anchored_and_ignores_selection_movement() {
        let selected_accept = "Would you like to run the following command?\n\n  cargo test\n\n\
            › 1. Yes, proceed\n  2. No, and tell Codex what to do differently\n\n\
            Press enter to confirm or esc to cancel";
        let selected_reject = selected_accept
            .replace("› 1. Yes", "  1. Yes")
            .replace("  2. No", "› 2. No");
        let quoted_history = format!("{selected_accept}\n\nThe command completed successfully.");

        let accept_fingerprint =
            approval_prompt_fingerprint(ApprovalPromptProvider::Codex, selected_accept)
                .expect("complete bottom prompt");
        let reject_fingerprint =
            approval_prompt_fingerprint(ApprovalPromptProvider::Codex, &selected_reject)
                .expect("selection may move");
        let shifted_history = format!("unrelated output scrolled above\n{selected_accept}");
        let shifted_fingerprint =
            approval_prompt_fingerprint(ApprovalPromptProvider::Codex, &shifted_history)
                .expect("history above the prompt is not prompt identity");
        let ascii_selection = selected_accept.replace('›', ">");
        let ascii_fingerprint =
            approval_prompt_fingerprint(ApprovalPromptProvider::Codex, &ascii_selection)
                .expect("ASCII selection glyph");

        assert_eq!(accept_fingerprint, reject_fingerprint);
        assert_eq!(accept_fingerprint, shifted_fingerprint);
        assert_eq!(accept_fingerprint, ascii_fingerprint);
        assert!(
            approval_prompt_fingerprint(ApprovalPromptProvider::Codex, &quoted_history).is_none()
        );
    }

    #[test]
    fn approval_prompt_classifier_normalizes_soft_wraps_and_rejects_custom_provider_names() {
        let narrow_claude = "Do you want to make this edit\n to src/main.rs?\n\n\
            ❯ 1. Yes, allow\n  2. No, and tell Claude what to do\n differently\n\n\
            Enter to confirm · Esc to cancel";

        assert!(detect_approval_prompt(
            ApprovalPromptProvider::ClaudeCode,
            narrow_claude
        ));
        assert!(!detect_approval_prompt(
            ApprovalPromptProvider::Unsupported,
            narrow_claude
        ));
    }

    #[test]
    fn approval_prompt_classifier_handles_sanitized_width_and_title_variants() {
        let codex_160 = "Would you like to grant these permissions?\n allow network for tests\n\
            › 1. Yes, proceed\n 2. No, and tell Codex what to do differently\n\
            Press enter to confirm or esc to cancel";
        let codex_80 = codex_160
            .replace("grant these permissions", "grant these\n permissions")
            .replace("what to do differently", "what to do\n differently");
        let codex_40 = codex_160
            .replace(
                "Would you like to grant these permissions?",
                "Would you like to\n grant these\n permissions?",
            )
            .replace(
                "tell Codex what to do differently",
                "tell Codex what\n to do differently",
            );

        let fingerprints = [codex_160, codex_80.as_str(), codex_40.as_str()].map(|screen| {
            approval_prompt_fingerprint(ApprovalPromptProvider::Codex, screen)
                .expect("width variant")
        });
        assert_eq!(fingerprints[0], fingerprints[1]);
        assert_eq!(fingerprints[0], fingerprints[2]);

        for title in [
            "Would you like to make the following edits?",
            "The shell command needs your approval.",
        ] {
            let prompt = format!(
                "{title}\n details\n› 1. Yes, proceed\n\
                 2. No, and tell Codex what to do differently\n\
                 Press enter to confirm or esc to cancel"
            );
            assert!(detect_approval_prompt(
                ApprovalPromptProvider::Codex,
                &prompt
            ));
        }
    }

    #[test]
    fn approval_prompt_selection_must_belong_to_the_bottom_prompt_block() {
        let prose = "> 1. Yes, proceed\n\
            Would you like to run the following command?\n command\n\
            1. Yes, proceed\n 2. No, and tell Codex what to do differently\n\
            Press enter to confirm or esc to cancel";

        assert!(!detect_approval_prompt(
            ApprovalPromptProvider::Codex,
            prose
        ));
    }

    #[test]
    fn approval_evidence_is_bottom_anchored_not_scattered_history() {
        let historical = "Would you like to run the following command?\n\
            output\nNo, and tell Codex what to do differently\n\
            Press enter to confirm or esc to cancel\nRunning tests...";
        let scattered = "Would you like to run the following command?\n\
            unrelated output\nRunning tests...\n\
            No, and tell Codex what to do differently\nStill running...";

        assert!(!has_approval_prompt_evidence(
            ApprovalPromptProvider::Codex,
            historical
        ));
        assert!(!has_approval_prompt_evidence(
            ApprovalPromptProvider::Codex,
            scattered
        ));
    }

    #[test]
    fn approval_wait_overlay_precedes_live_hook_state_but_not_terminal_state() {
        assert_eq!(
            compose_window_state_with_approval_wait(
                WindowState::Running,
                WindowPreset::Codex,
                Some(WindowState::Running),
                true,
                true,
            ),
            WindowState::Waiting
        );
        assert_eq!(
            compose_window_state_with_approval_wait(
                WindowState::Running,
                WindowPreset::Shell,
                None,
                false,
                true,
            ),
            WindowState::Running
        );
        assert_eq!(
            compose_window_state_with_approval_wait(
                WindowState::Stopped,
                WindowPreset::Codex,
                Some(WindowState::Running),
                true,
                true,
            ),
            WindowState::Stopped
        );
        assert_eq!(
            compose_window_state_with_approval_wait(
                WindowState::Error,
                WindowPreset::Codex,
                Some(WindowState::Running),
                true,
                true,
            ),
            WindowState::Error
        );
    }

    #[test]
    fn approval_resolution_input_only_accepts_submit_or_standalone_cancel() {
        assert!(is_approval_resolution_input("\r"));
        assert!(is_approval_resolution_input("\n"));
        assert!(is_approval_resolution_input("\r\n"));
        assert!(is_approval_resolution_input("\u{1b}"));
        assert!(!is_approval_resolution_input("1"));
        assert!(!is_approval_resolution_input("\u{1b}[A"));
        assert!(is_approval_resolution_input("1\r"));
        assert!(is_approval_resolution_input("yes\n"));
        assert!(!is_approval_resolution_input("yes"));
    }
}
