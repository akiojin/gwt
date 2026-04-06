//! Keybind registry — Ctrl+G prefix system with auto-collected help.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::input::voice::VoiceInputMessage;
use crate::message::Message;
use crate::model::ManagementTab;

/// Timeout for the Ctrl+G prefix sequence.
const PREFIX_TIMEOUT: Duration = Duration::from_secs(2);

/// State machine for the Ctrl+G prefix system.
#[derive(Debug, Clone, Default)]
pub enum PrefixState {
    /// Waiting for Ctrl+G.
    #[default]
    Idle,
    /// Ctrl+G pressed, waiting for the second key.
    Active { since: Instant },
}

impl PrefixState {
    /// Check if the prefix has timed out.
    pub fn is_expired(&self) -> bool {
        match self {
            Self::Idle => false,
            Self::Active { since } => since.elapsed() > PREFIX_TIMEOUT,
        }
    }
}

/// Logical grouping for a registered keybinding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeybindingCategory {
    Global,
    Sessions,
    Management,
    Input,
}

impl KeybindingCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::Sessions => "Sessions",
            Self::Management => "Management",
            Self::Input => "Input",
        }
    }
}

/// A registered keybinding.
#[derive(Debug, Clone)]
pub struct Keybinding {
    /// Display string for the key combo (e.g. "Ctrl+G, g").
    pub keys: String,
    /// Description of what this binding does.
    pub description: String,
    /// Logical grouping for the help overlay.
    pub category: KeybindingCategory,
}

/// Registry of all keybindings, also drives the prefix state machine.
#[derive(Debug)]
pub struct KeybindRegistry {
    pub prefix_state: PrefixState,
    bindings: Vec<Keybinding>,
    /// Track last Ctrl+C time for double-tap quit.
    last_ctrl_c: Option<Instant>,
}

/// Window for double-tap Ctrl+C detection.
const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(500);

impl Default for KeybindRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl KeybindRegistry {
    pub fn new() -> Self {
        let bindings = vec![
            Keybinding {
                keys: "Ctrl+G, g".into(),
                description: "Toggle management panel".into(),
                category: KeybindingCategory::Global,
            },
            Keybinding {
                keys: "Ctrl+G, Tab".into(),
                description: "Focus next pane".into(),
                category: KeybindingCategory::Global,
            },
            Keybinding {
                keys: "Ctrl+G, Shift+Tab".into(),
                description: "Focus previous pane".into(),
                category: KeybindingCategory::Global,
            },
            Keybinding {
                keys: "Ctrl+G, ]".into(),
                description: "Next session".into(),
                category: KeybindingCategory::Sessions,
            },
            Keybinding {
                keys: "Ctrl+G, [".into(),
                description: "Previous session".into(),
                category: KeybindingCategory::Sessions,
            },
            Keybinding {
                keys: "Ctrl+G, 1-9".into(),
                description: "Switch to session N".into(),
                category: KeybindingCategory::Sessions,
            },
            Keybinding {
                keys: "Ctrl+G, z".into(),
                description: "Toggle Tab/Grid layout".into(),
                category: KeybindingCategory::Sessions,
            },
            Keybinding {
                keys: "Ctrl+G, c".into(),
                description: "New shell session".into(),
                category: KeybindingCategory::Sessions,
            },
            Keybinding {
                keys: "Ctrl+G, x".into(),
                description: "Close active session".into(),
                category: KeybindingCategory::Sessions,
            },
            Keybinding {
                keys: "Ctrl+G, q".into(),
                description: "Quit".into(),
                category: KeybindingCategory::Global,
            },
            Keybinding {
                keys: "Ctrl+G, v".into(),
                description: "Start voice input".into(),
                category: KeybindingCategory::Input,
            },
            Keybinding {
                keys: "Ctrl+G, a".into(),
                description: "Convert active agent session".into(),
                category: KeybindingCategory::Sessions,
            },
            Keybinding {
                keys: "Ctrl+G, p".into(),
                description: "Paste file paths".into(),
                category: KeybindingCategory::Input,
            },
            Keybinding {
                keys: "Ctrl+G, ?".into(),
                description: "Show help".into(),
                category: KeybindingCategory::Global,
            },
            Keybinding {
                keys: "Ctrl+G, b".into(),
                description: "Switch to Branches tab".into(),
                category: KeybindingCategory::Management,
            },
            Keybinding {
                keys: "Ctrl+G, s".into(),
                description: "Switch to Settings tab".into(),
                category: KeybindingCategory::Management,
            },
            Keybinding {
                keys: "Ctrl+G, i".into(),
                description: "Switch to Issues tab".into(),
                category: KeybindingCategory::Management,
            },
            Keybinding {
                keys: "Ctrl+C, Ctrl+C".into(),
                description: "Quit".into(),
                category: KeybindingCategory::Global,
            },
        ];

        Self {
            prefix_state: PrefixState::Idle,
            bindings,
            last_ctrl_c: None,
        }
    }

    /// Get all bindings for help display.
    pub fn all_bindings(&self) -> &[Keybinding] {
        &self.bindings
    }

    /// Process a key event through the prefix state machine.
    /// Returns `Some(Message)` if the key was consumed, `None` if it should be forwarded.
    ///
    /// When `terminal_focused` is true, Ctrl+C double-tap quit is disabled so
    /// that every Ctrl+C reaches the PTY as SIGINT.  Use `Ctrl+G, q` to quit.
    pub fn process_key(&mut self, key: KeyEvent) -> Option<Message> {
        self.process_key_with_focus(key, false)
    }

    /// Process a key event, optionally suppressing double-tap Ctrl+C quit
    /// when a terminal session has focus.
    pub fn process_key_with_focus(
        &mut self,
        key: KeyEvent,
        terminal_focused: bool,
    ) -> Option<Message> {
        // Check for timeout
        if self.prefix_state.is_expired() {
            self.prefix_state = PrefixState::Idle;
        }

        // Ctrl+C double-tap quit — disabled when terminal has focus so that
        // every Ctrl+C reaches the PTY as SIGINT.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if !terminal_focused {
                if let Some(last) = self.last_ctrl_c {
                    if last.elapsed() < DOUBLE_TAP_WINDOW {
                        self.last_ctrl_c = None;
                        return Some(Message::Quit);
                    }
                }
                self.last_ctrl_c = Some(Instant::now());
            }
            return None; // Forward to PTY (or non-terminal handler)
        }
        // Any non-Ctrl+C key resets the double-tap tracker
        self.last_ctrl_c = None;

        match &self.prefix_state {
            PrefixState::Idle => {
                // Check for Ctrl+G
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('g') {
                    self.prefix_state = PrefixState::Active {
                        since: Instant::now(),
                    };
                    return Some(Message::Tick); // Consumed, no action yet
                }
                None // Not a prefix key, forward to PTY
            }
            PrefixState::Active { .. } => {
                self.prefix_state = PrefixState::Idle;
                match key.code {
                    KeyCode::Char('g') => Some(Message::ToggleLayer),
                    KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        Some(Message::CycleFocusBackward)
                    }
                    KeyCode::Tab => Some(Message::CycleFocusForward),
                    KeyCode::BackTab => Some(Message::CycleFocusBackward),
                    KeyCode::Char(']') => Some(Message::NextSession),
                    KeyCode::Char('[') => Some(Message::PrevSession),
                    KeyCode::Char('z') => Some(Message::ToggleSessionLayout),
                    KeyCode::Char('c') => Some(Message::NewShell),
                    KeyCode::Char('x') => Some(Message::CloseSession),
                    KeyCode::Char('q') => Some(Message::Quit),
                    KeyCode::Char(n) if n.is_ascii_digit() && n != '0' => {
                        let idx = (n as usize) - ('1' as usize);
                        Some(Message::SwitchSession(idx))
                    }
                    KeyCode::Char('v') => Some(Message::Voice(VoiceInputMessage::StartRecording)),
                    KeyCode::Char('a') => Some(Message::OpenSessionConversion),
                    KeyCode::Char('b') => {
                        Some(Message::SwitchManagementTab(ManagementTab::Branches))
                    }
                    KeyCode::Char('s') => {
                        Some(Message::SwitchManagementTab(ManagementTab::Settings))
                    }
                    KeyCode::Char('i') => Some(Message::SwitchManagementTab(ManagementTab::Issues)),
                    KeyCode::Char('p') => Some(Message::PasteFiles),
                    KeyCode::Char('?') => Some(Message::ToggleHelp),
                    KeyCode::Esc => Some(Message::Tick), // Cancel prefix
                    _ => None,                           // Unknown, discard
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn idle_ctrl_g_activates_prefix() {
        let mut reg = KeybindRegistry::new();
        let result = reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        assert!(result.is_some());
        assert!(matches!(reg.prefix_state, PrefixState::Active { .. }));
    }

    #[test]
    fn prefix_g_toggles_layer() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('g'), KeyModifiers::NONE));
        assert!(matches!(result, Some(Message::ToggleLayer)));
        assert!(matches!(reg.prefix_state, PrefixState::Idle));
    }

    #[test]
    fn prefix_q_quits() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(matches!(result, Some(Message::Quit)));
    }

    #[test]
    fn prefix_bracket_navigates_sessions() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char(']'), KeyModifiers::NONE));
        assert!(matches!(result, Some(Message::NextSession)));

        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('['), KeyModifiers::NONE));
        assert!(matches!(result, Some(Message::PrevSession)));
    }

    #[test]
    fn prefix_number_switches_session() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('3'), KeyModifiers::NONE));
        assert!(matches!(result, Some(Message::SwitchSession(2))));
    }

    #[test]
    fn prefix_z_toggles_layout() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('z'), KeyModifiers::NONE));
        assert!(matches!(result, Some(Message::ToggleSessionLayout)));
    }

    #[test]
    fn prefix_v_starts_voice_recording() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('v'), KeyModifiers::NONE));
        assert!(matches!(
            result,
            Some(Message::Voice(VoiceInputMessage::StartRecording))
        ));
    }

    #[test]
    fn prefix_question_toggles_help() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(matches!(result, Some(Message::ToggleHelp)));
    }

    #[test]
    fn prefix_esc_cancels() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(result.is_some()); // Tick — consumed but no action
        assert!(matches!(reg.prefix_state, PrefixState::Idle));
    }

    #[test]
    fn non_prefix_key_returns_none() {
        let mut reg = KeybindRegistry::new();
        let result = reg.process_key(key(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(result.is_none());
    }

    #[test]
    fn all_bindings_not_empty() {
        let reg = KeybindRegistry::new();
        assert!(!reg.all_bindings().is_empty());
        assert!(reg
            .all_bindings()
            .iter()
            .any(|binding| binding.category == KeybindingCategory::Management));
    }

    #[test]
    fn expired_prefix_resets_to_idle() {
        let mut reg = KeybindRegistry::new();
        reg.prefix_state = PrefixState::Active {
            since: Instant::now() - Duration::from_secs(3),
        };
        let result = reg.process_key(key(KeyCode::Char('g'), KeyModifiers::NONE));
        // Should treat as idle — 'g' without ctrl is not a prefix trigger
        assert!(result.is_none());
    }

    #[test]
    fn prefix_management_shortcuts() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('b'), KeyModifiers::NONE));
        assert!(matches!(
            result,
            Some(Message::SwitchManagementTab(ManagementTab::Branches))
        ));

        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('i'), KeyModifiers::NONE));
        assert!(matches!(
            result,
            Some(Message::SwitchManagementTab(ManagementTab::Issues))
        ));
    }

    #[test]
    fn prefix_p_paste_files() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('p'), KeyModifiers::NONE));
        assert!(matches!(result, Some(Message::PasteFiles)));
    }

    #[test]
    fn prefix_tab_cycles_focus_forward_even_when_terminal_is_focused() {
        let mut reg = KeybindRegistry::new();
        reg.process_key_with_focus(key(KeyCode::Char('g'), KeyModifiers::CONTROL), true);
        let result = reg.process_key_with_focus(key(KeyCode::Tab, KeyModifiers::NONE), true);
        assert!(matches!(result, Some(Message::CycleFocusForward)));
    }

    #[test]
    fn prefix_shift_tab_cycles_focus_backward() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Tab, KeyModifiers::SHIFT));
        assert!(matches!(result, Some(Message::CycleFocusBackward)));

        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert!(matches!(result, Some(Message::CycleFocusBackward)));
    }

    #[test]
    fn prefix_a_opens_session_conversion() {
        let mut reg = KeybindRegistry::new();
        reg.process_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
        let result = reg.process_key(key(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(matches!(result, Some(Message::OpenSessionConversion)));
    }

    #[test]
    fn all_bindings_include_registered_shortcuts() {
        let reg = KeybindRegistry::new();
        let registered: Vec<&str> = reg
            .all_bindings()
            .iter()
            .map(|binding| binding.keys.as_str())
            .collect();

        for expected in [
            "Ctrl+G, g",
            "Ctrl+G, c",
            "Ctrl+G, ?",
            "Ctrl+G, 1-9",
            "Ctrl+C, Ctrl+C",
        ] {
            assert!(
                registered.contains(&expected),
                "expected binding registry to contain {expected}"
            );
        }
    }

    #[test]
    fn all_bindings_exclude_unregistered_shortcuts() {
        let reg = KeybindRegistry::new();
        let registered: Vec<&str> = reg
            .all_bindings()
            .iter()
            .map(|binding| binding.keys.as_str())
            .collect();
        assert!(!registered.contains(&"Ctrl+G, y"));
    }
}
