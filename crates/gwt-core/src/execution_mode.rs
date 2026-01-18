//! Execution mode detection for gwt
//!
//! Determines whether gwt should run in Single or Multi mode based on
//! the runtime environment (tmux availability).

use crate::tmux::detector::is_inside_tmux;

/// Execution mode for gwt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TmuxMode {
    /// Single agent mode (traditional, outside tmux)
    #[default]
    Single,
    /// Multi agent mode (inside tmux, supports parallel agents)
    Multi,
}

impl TmuxMode {
    /// Detect the execution mode based on the current environment
    ///
    /// Returns `Multi` if running inside a tmux session, `Single` otherwise.
    pub fn detect() -> Self {
        if is_inside_tmux() {
            TmuxMode::Multi
        } else {
            TmuxMode::Single
        }
    }

    /// Check if this is multi-agent mode
    pub fn is_multi(&self) -> bool {
        matches!(self, TmuxMode::Multi)
    }

    /// Check if this is single-agent mode
    pub fn is_single(&self) -> bool {
        matches!(self, TmuxMode::Single)
    }

    /// Get a human-readable description of the mode
    pub fn description(&self) -> &'static str {
        match self {
            TmuxMode::Single => "Single Agent Mode",
            TmuxMode::Multi => "Multi Agent Mode (tmux)",
        }
    }
}

impl std::fmt::Display for TmuxMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TmuxMode::Single => write!(f, "Single"),
            TmuxMode::Multi => write!(f, "Multi"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_mode_single_outside_tmux() {
        // Save original value
        let original = std::env::var("TMUX").ok();

        // Remove TMUX environment variable
        std::env::remove_var("TMUX");
        let mode = TmuxMode::detect();
        assert_eq!(mode, TmuxMode::Single);

        // Restore original value
        if let Some(val) = original {
            std::env::set_var("TMUX", val);
        }
    }

    #[test]
    fn test_execution_mode_multi_inside_tmux() {
        // Save original value
        let original = std::env::var("TMUX").ok();

        // Set TMUX environment variable
        std::env::set_var("TMUX", "/tmp/tmux-1000/default,12345,0");
        let mode = TmuxMode::detect();
        assert_eq!(mode, TmuxMode::Multi);

        // Restore original value
        if let Some(val) = original {
            std::env::set_var("TMUX", val);
        } else {
            std::env::remove_var("TMUX");
        }
    }

    #[test]
    fn test_execution_mode_is_multi() {
        assert!(TmuxMode::Multi.is_multi());
        assert!(!TmuxMode::Single.is_multi());
    }

    #[test]
    fn test_execution_mode_is_single() {
        assert!(TmuxMode::Single.is_single());
        assert!(!TmuxMode::Multi.is_single());
    }

    #[test]
    fn test_execution_mode_display() {
        assert_eq!(TmuxMode::Single.to_string(), "Single");
        assert_eq!(TmuxMode::Multi.to_string(), "Multi");
    }

    #[test]
    fn test_execution_mode_description() {
        assert!(TmuxMode::Single.description().contains("Single"));
        assert!(TmuxMode::Multi.description().contains("Multi"));
    }

    #[test]
    fn test_execution_mode_default() {
        assert_eq!(TmuxMode::default(), TmuxMode::Single);
    }
}
