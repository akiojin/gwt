use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::state::PrefixState;

/// Timeout for Ctrl+G prefix mode (2 seconds per design doc).
const PREFIX_TIMEOUT: Duration = Duration::from_secs(2);

/// Direction for pane focus switching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Actions produced by the key binding state machine.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyAction {
    /// Create a new shell Window.
    NewShellWindow,
    /// Open agent launch dialog.
    NewAgentWindow,
    /// Switch to Window by number (0-indexed).
    SwitchTab(usize),
    /// Next Window.
    NextWindow,
    /// Previous Window.
    PrevWindow,
    /// Close current Window.
    CloseWindow,
    /// Vertical split in current Window.
    VerticalSplit,
    /// Horizontal split in current Window.
    HorizontalSplit,
    /// Move focus to adjacent pane.
    FocusPane(Direction),
    /// Close active pane.
    ClosePane,
    /// Toggle pane zoom.
    ZoomPane,
    /// Toggle management panel.
    ToggleManagement,
    /// Enter keyboard scroll mode.
    ScrollMode,
    /// Quit gwt.
    Quit,
    /// Pass key through to PTY.
    Passthrough(KeyEvent),
    /// No action (consumed key, e.g. entering prefix mode).
    None,
}

/// Process a key event through the prefix state machine.
///
/// Returns the action to take and mutates the prefix state.
pub fn process_key(state: &mut PrefixState, key: KeyEvent) -> KeyAction {
    match state {
        PrefixState::Idle => {
            // Check for Ctrl+G
            if key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL) {
                *state = PrefixState::Active(Instant::now());
                return KeyAction::None;
            }
            KeyAction::Passthrough(key)
        }
        PrefixState::Active(activated_at) => {
            // Check timeout
            if activated_at.elapsed() >= PREFIX_TIMEOUT {
                *state = PrefixState::Idle;
                return KeyAction::Passthrough(key);
            }

            *state = PrefixState::Idle;

            match key.code {
                // Cancel prefix
                KeyCode::Esc => KeyAction::None,
                // Ctrl+G again = toggle management panel
                KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    KeyAction::ToggleManagement
                }
                // Window operations
                KeyCode::Char('c') => KeyAction::NewShellWindow,
                KeyCode::Char('n') => KeyAction::NewAgentWindow,
                KeyCode::Char(']') => KeyAction::NextWindow,
                KeyCode::Char('[') => KeyAction::PrevWindow,
                KeyCode::Char('&') => KeyAction::CloseWindow,
                // Tab switching by number
                KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                    KeyAction::SwitchTab((c as usize) - ('1' as usize))
                }
                // Pane operations
                KeyCode::Char('v') => KeyAction::VerticalSplit,
                KeyCode::Char('h') => KeyAction::HorizontalSplit,
                KeyCode::Char('x') => KeyAction::ClosePane,
                KeyCode::Char('z') => KeyAction::ZoomPane,
                // Focus movement
                KeyCode::Left => KeyAction::FocusPane(Direction::Left),
                KeyCode::Right => KeyAction::FocusPane(Direction::Right),
                KeyCode::Up => KeyAction::FocusPane(Direction::Up),
                KeyCode::Down => KeyAction::FocusPane(Direction::Down),
                // Scroll mode
                KeyCode::PageUp => KeyAction::ScrollMode,
                // Quit
                KeyCode::Char('q') => KeyAction::Quit,
                // Unknown prefix command — discard
                _ => KeyAction::None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn test_passthrough_normal_key() {
        let mut state = PrefixState::Idle;
        let action = process_key(&mut state, key(KeyCode::Char('a')));
        assert!(matches!(action, KeyAction::Passthrough(_)));
        assert_eq!(state, PrefixState::Idle);
    }

    #[test]
    fn test_ctrl_g_enters_prefix_mode() {
        let mut state = PrefixState::Idle;
        let action = process_key(&mut state, ctrl_key('g'));
        assert_eq!(action, KeyAction::None);
        assert!(matches!(state, PrefixState::Active(_)));
    }

    #[test]
    fn test_prefix_new_shell() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char('c')));
        assert_eq!(action, KeyAction::NewShellWindow);
        assert_eq!(state, PrefixState::Idle);
    }

    #[test]
    fn test_prefix_new_agent() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char('n')));
        assert_eq!(action, KeyAction::NewAgentWindow);
    }

    #[test]
    fn test_prefix_next_window() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char(']')));
        assert_eq!(action, KeyAction::NextWindow);
    }

    #[test]
    fn test_prefix_prev_window() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char('[')));
        assert_eq!(action, KeyAction::PrevWindow);
    }

    #[test]
    fn test_prefix_switch_tab_by_number() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char('3')));
        assert_eq!(action, KeyAction::SwitchTab(2)); // 0-indexed
    }

    #[test]
    fn test_prefix_vertical_split() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char('v')));
        assert_eq!(action, KeyAction::VerticalSplit);
    }

    #[test]
    fn test_prefix_horizontal_split() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char('h')));
        assert_eq!(action, KeyAction::HorizontalSplit);
    }

    #[test]
    fn test_prefix_toggle_management() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, ctrl_key('g'));
        assert_eq!(action, KeyAction::ToggleManagement);
    }

    #[test]
    fn test_prefix_escape_cancels() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Esc));
        assert_eq!(action, KeyAction::None);
        assert_eq!(state, PrefixState::Idle);
    }

    #[test]
    fn test_prefix_timeout_passes_through() {
        let mut state = PrefixState::Active(Instant::now() - Duration::from_secs(3));
        let action = process_key(&mut state, key(KeyCode::Char('n')));
        assert!(matches!(action, KeyAction::Passthrough(_)));
        assert_eq!(state, PrefixState::Idle);
    }

    #[test]
    fn test_prefix_quit() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char('q')));
        assert_eq!(action, KeyAction::Quit);
    }

    #[test]
    fn test_prefix_scroll_mode() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::PageUp));
        assert_eq!(action, KeyAction::ScrollMode);
    }

    #[test]
    fn test_prefix_focus_pane_arrows() {
        let mut state = PrefixState::Active(Instant::now());
        assert_eq!(
            process_key(&mut state, key(KeyCode::Left)),
            KeyAction::FocusPane(Direction::Left)
        );

        let mut state = PrefixState::Active(Instant::now());
        assert_eq!(
            process_key(&mut state, key(KeyCode::Right)),
            KeyAction::FocusPane(Direction::Right)
        );
    }

    #[test]
    fn test_prefix_close_pane() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char('x')));
        assert_eq!(action, KeyAction::ClosePane);
    }

    #[test]
    fn test_prefix_zoom_pane() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char('z')));
        assert_eq!(action, KeyAction::ZoomPane);
    }

    #[test]
    fn test_prefix_unknown_key_returns_none() {
        let mut state = PrefixState::Active(Instant::now());
        let action = process_key(&mut state, key(KeyCode::Char('?')));
        assert_eq!(action, KeyAction::None);
        assert_eq!(state, PrefixState::Idle);
    }
}
