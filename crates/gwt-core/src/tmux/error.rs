//! tmux-specific error types
//!
//! Error codes are categorized as E6xxx for tmux operations.

use thiserror::Error;

/// Result type alias for tmux operations
pub type TmuxResult<T> = std::result::Result<T, TmuxError>;

/// Error type for tmux operations
#[derive(Error, Debug)]
pub enum TmuxError {
    /// tmux is not installed on the system
    #[error("[E6001] tmux is not installed")]
    NotInstalled,

    /// tmux version is too old (requires 2.0+)
    #[error("[E6002] tmux version {version} is too old (requires 2.0+)")]
    VersionTooOld { version: String },

    /// Failed to parse tmux version
    #[error("[E6003] Failed to parse tmux version: {output}")]
    VersionParseFailed { output: String },

    /// tmux command execution failed
    #[error("[E6004] tmux command failed: {command}: {reason}")]
    CommandFailed { command: String, reason: String },

    /// Session not found
    #[error("[E6005] tmux session not found: {name}")]
    SessionNotFound { name: String },

    /// Session already exists
    #[error("[E6006] tmux session already exists: {name}")]
    SessionAlreadyExists { name: String },

    /// Failed to create session
    #[error("[E6007] Failed to create tmux session: {name}: {reason}")]
    SessionCreateFailed { name: String, reason: String },

    /// Failed to destroy session
    #[error("[E6008] Failed to destroy tmux session: {name}: {reason}")]
    SessionDestroyFailed { name: String, reason: String },

    /// Pane not found
    #[error("[E6009] tmux pane not found: {pane_id}")]
    PaneNotFound { pane_id: String },

    /// Failed to create pane
    #[error("[E6010] Failed to create tmux pane: {reason}")]
    PaneCreateFailed { reason: String },

    /// Failed to kill pane
    #[error("[E6011] Failed to kill tmux pane: {pane_id}: {reason}")]
    PaneKillFailed { pane_id: String, reason: String },

    /// Not inside tmux environment
    #[error("[E6012] Not running inside tmux")]
    NotInsideTmux,

    /// IO error
    #[error("[E6013] IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tmux_error_display() {
        let err = TmuxError::NotInstalled;
        assert!(err.to_string().contains("E6001"));
        assert!(err.to_string().contains("not installed"));
    }

    #[test]
    fn test_tmux_error_version_too_old() {
        let err = TmuxError::VersionTooOld {
            version: "1.9".to_string(),
        };
        assert!(err.to_string().contains("E6002"));
        assert!(err.to_string().contains("1.9"));
        assert!(err.to_string().contains("2.0"));
    }

    #[test]
    fn test_tmux_error_command_failed() {
        let err = TmuxError::CommandFailed {
            command: "list-sessions".to_string(),
            reason: "server not found".to_string(),
        };
        assert!(err.to_string().contains("E6004"));
        assert!(err.to_string().contains("list-sessions"));
    }
}
