//! Keybind registry — Ctrl+G prefix system with auto-collected help.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

/// A registered keybinding.
#[derive(Debug, Clone)]
pub struct Keybinding {
    /// Display string for the key combo (e.g. "Ctrl+G, g").
    pub keys: String,
    /// Description of what this binding does.
    pub description: String,
}

/// Registry of all keybindings, also drives the prefix state machine.
#[derive(Debug)]
pub struct KeybindRegistry {
    pub prefix_state: PrefixState,
    bindings: Vec<Keybinding>,
}

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
            },
            Keybinding {
                keys: "Ctrl+G, ]".into(),
                description: "Next session".into(),
            },
            Keybinding {
                keys: "Ctrl+G, [".into(),
                description: "Previous session".into(),
            },
            Keybinding {
                keys: "Ctrl+G, 1-9".into(),
                description: "Switch to session N".into(),
            },
            Keybinding {
                keys: "Ctrl+G, z".into(),
                description: "Toggle Tab/Grid layout".into(),
            },
            Keybinding {
                keys: "Ctrl+G, c".into(),
                description: "New shell session".into(),
            },
            Keybinding {
                keys: "Ctrl+G, x".into(),
                description: "Close active session".into(),
            },
            Keybinding {
                keys: "Ctrl+G, q".into(),
                description: "Quit".into(),
            },
            Keybinding {
                keys: "Ctrl+G, ?".into(),
                description: "Show help".into(),
            },
        ];

        Self {
            prefix_state: PrefixState::Idle,
            bindings,
        }
    }

    /// Get all bindings for help display.
    pub fn all_bindings(&self) -> &[Keybinding] {
        &self.bindings
    }

    /// Process a key event through the prefix state machine.
    /// Returns `Some(Message)` if the key was consumed, `None` if it should be forwarded.
    pub fn process_key(&mut self, key: KeyEvent) -> Option<Message> {
        // Check for timeout
        if self.prefix_state.is_expired() {
            self.prefix_state = PrefixState::Idle;
        }

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
                    KeyCode::Char('b') => {
                        Some(Message::SwitchManagementTab(ManagementTab::Branches))
                    }
                    KeyCode::Char('s') => Some(Message::SwitchManagementTab(ManagementTab::Specs)),
                    KeyCode::Char('i') => Some(Message::SwitchManagementTab(ManagementTab::Issues)),
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
}
