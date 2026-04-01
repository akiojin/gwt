//! Ctrl+G prefix keybind system
//!
//! Keybindings follow a Ctrl+G prefix pattern (like tmux's Ctrl+B):
//! - Ctrl+G, then a second key triggers an action
//! - Timeout after 1s returns to normal mode

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const PREFIX_TIMEOUT: Duration = Duration::from_secs(1);

/// The state of the prefix key system.
#[derive(Debug, Clone, Default)]
pub enum PrefixState {
    /// Normal mode: waiting for Ctrl+G
    #[default]
    Normal,
    /// Prefix received: waiting for the second key
    Prefix { at: Instant },
}

/// Actions produced by the keybind system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// No action (key consumed or prefix waiting)
    None,
    /// Forward the key to the active pane / screen
    Forward(KeyEvent),
    /// Toggle between Main and Management layers
    ToggleLayer,
    /// Next session tab
    NextSession,
    /// Previous session tab
    PrevSession,
    /// Switch to session by 1-based number (1-9)
    SwitchSession(usize),
    /// Close current session
    CloseSession,
    /// New shell
    NewShell,
    /// Open wizard
    OpenWizard,
    /// Show help
    ShowHelp,
    /// Quit
    Quit,
}

/// Process a key event through the prefix system.
///
/// Returns the resulting action. Mutates `state` in place.
pub fn process_key(state: &mut PrefixState, key: KeyEvent) -> KeyAction {
    match state {
        PrefixState::Normal => {
            if is_ctrl_g(&key) {
                *state = PrefixState::Prefix { at: Instant::now() };
                KeyAction::None
            } else {
                KeyAction::Forward(key)
            }
        }
        PrefixState::Prefix { at } => {
            let elapsed = at.elapsed();
            *state = PrefixState::Normal;

            // Timeout check
            if elapsed > PREFIX_TIMEOUT {
                return KeyAction::Forward(key);
            }

            // Second key after Ctrl+G
            match key.code {
                // Ctrl+G, Ctrl+G → toggle layer
                KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    KeyAction::ToggleLayer
                }
                // Ctrl+G, ] → next session
                KeyCode::Char(']') => KeyAction::NextSession,
                // Ctrl+G, [ → prev session
                KeyCode::Char('[') => KeyAction::PrevSession,
                // Ctrl+G, 1-9 → switch session
                KeyCode::Char(c @ '1'..='9') => {
                    KeyAction::SwitchSession((c as usize) - ('0' as usize))
                }
                // Ctrl+G, & → close session
                KeyCode::Char('&') => KeyAction::CloseSession,
                // Ctrl+G, c → new shell
                KeyCode::Char('c') => KeyAction::NewShell,
                // Ctrl+G, n → open wizard
                KeyCode::Char('n') => KeyAction::OpenWizard,
                // Ctrl+G, ? → help
                KeyCode::Char('?') => KeyAction::ShowHelp,
                // Ctrl+G, q → no longer used for quit (Ctrl+C double-tap instead)
                KeyCode::Char('q') => KeyAction::Forward(key),
                // Unknown second key → discard prefix, forward key
                _ => KeyAction::Forward(key),
            }
        }
    }
}

/// Check if a key event is Ctrl+G.
fn is_ctrl_g(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Check if a key event is Ctrl+C.
pub fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn ctrl_g() -> KeyEvent {
        make_key(KeyCode::Char('g'), KeyModifiers::CONTROL)
    }

    fn ctrl_c() -> KeyEvent {
        make_key(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    fn plain(c: char) -> KeyEvent {
        make_key(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn normal_key_is_forwarded() {
        let mut state = PrefixState::Normal;
        let key = plain('a');
        let action = process_key(&mut state, key);
        assert_eq!(action, KeyAction::Forward(key));
    }

    #[test]
    fn ctrl_g_enters_prefix() {
        let mut state = PrefixState::Normal;
        let action = process_key(&mut state, ctrl_g());
        assert_eq!(action, KeyAction::None);
        assert!(matches!(state, PrefixState::Prefix { .. }));
    }

    #[test]
    fn ctrl_g_ctrl_g_toggles_layer() {
        let mut state = PrefixState::Normal;
        process_key(&mut state, ctrl_g());
        let action = process_key(&mut state, ctrl_g());
        assert_eq!(action, KeyAction::ToggleLayer);
        assert!(matches!(state, PrefixState::Normal));
    }

    #[test]
    fn ctrl_g_bracket_navigates_sessions() {
        let mut state = PrefixState::Normal;
        process_key(&mut state, ctrl_g());
        assert_eq!(process_key(&mut state, plain(']')), KeyAction::NextSession);

        process_key(&mut state, ctrl_g());
        assert_eq!(process_key(&mut state, plain('[')), KeyAction::PrevSession);
    }

    #[test]
    fn ctrl_g_number_switches_session() {
        let mut state = PrefixState::Normal;
        process_key(&mut state, ctrl_g());
        assert_eq!(
            process_key(&mut state, plain('3')),
            KeyAction::SwitchSession(3)
        );
    }

    #[test]
    fn ctrl_g_c_new_shell() {
        let mut state = PrefixState::Normal;
        process_key(&mut state, ctrl_g());
        assert_eq!(process_key(&mut state, plain('c')), KeyAction::NewShell);
    }

    #[test]
    fn ctrl_g_ampersand_close_session() {
        let mut state = PrefixState::Normal;
        process_key(&mut state, ctrl_g());
        assert_eq!(process_key(&mut state, plain('&')), KeyAction::CloseSession);
    }

    #[test]
    fn ctrl_g_q_forwards() {
        // Ctrl+G, q no longer quits — forwards to screen instead
        let mut state = PrefixState::Normal;
        process_key(&mut state, ctrl_g());
        assert!(matches!(
            process_key(&mut state, plain('q')),
            KeyAction::Forward(_)
        ));
    }

    #[test]
    fn unknown_second_key_forwards() {
        let mut state = PrefixState::Normal;
        process_key(&mut state, ctrl_g());
        let key = plain('z');
        assert_eq!(process_key(&mut state, key), KeyAction::Forward(key));
    }

    #[test]
    fn is_ctrl_c_check() {
        assert!(is_ctrl_c(&ctrl_c()));
        assert!(!is_ctrl_c(&plain('c')));
    }

    #[test]
    fn message_conversion_covers_all_actions() {
        // Ensure KeyAction variants compile
        let actions = vec![
            KeyAction::None,
            KeyAction::Forward(plain('x')),
            KeyAction::ToggleLayer,
            KeyAction::NextSession,
            KeyAction::PrevSession,
            KeyAction::SwitchSession(1),
            KeyAction::CloseSession,
            KeyAction::NewShell,
            KeyAction::OpenWizard,
            KeyAction::ShowHelp,
            KeyAction::Quit,
        ];
        assert_eq!(actions.len(), 11);
    }
}
