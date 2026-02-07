//! Terminal mode configuration for gwt
//!
//! Determines whether agents run in the built-in terminal emulator
//! (default) or could be configured for alternative backends.

use serde::{Deserialize, Serialize};

/// Terminal mode for agent pane management.
///
/// Determines whether agents run in the built-in terminal emulator
/// or in tmux panes (legacy mode).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalMode {
    /// Built-in terminal emulator (default)
    #[default]
    Builtin,
    /// tmux-based pane management (legacy)
    Tmux,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_mode_default_is_builtin() {
        assert_eq!(TerminalMode::default(), TerminalMode::Builtin);
    }

    #[test]
    fn test_terminal_mode_serialize_deserialize() {
        let builtin = TerminalMode::Builtin;
        let json = serde_json::to_string(&builtin).expect("serialize Builtin");
        let deserialized: TerminalMode = serde_json::from_str(&json).expect("deserialize Builtin");
        assert_eq!(deserialized, TerminalMode::Builtin);

        let tmux = TerminalMode::Tmux;
        let json = serde_json::to_string(&tmux).expect("serialize Tmux");
        let deserialized: TerminalMode = serde_json::from_str(&json).expect("deserialize Tmux");
        assert_eq!(deserialized, TerminalMode::Tmux);
    }

    #[test]
    fn test_terminal_mode_equality() {
        assert_eq!(TerminalMode::Builtin, TerminalMode::Builtin);
        assert_eq!(TerminalMode::Tmux, TerminalMode::Tmux);
        assert_ne!(TerminalMode::Builtin, TerminalMode::Tmux);
    }

    #[test]
    fn test_terminal_mode_clone() {
        let mode = TerminalMode::Builtin;
        let cloned = mode.clone();
        assert_eq!(mode, cloned);
    }

    #[test]
    fn test_terminal_mode_debug() {
        let mode = TerminalMode::Builtin;
        let debug_str = format!("{:?}", mode);
        assert!(debug_str.contains("Builtin"));
    }
}
