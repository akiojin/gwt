//! Terminal-related error types
//!
//! Error codes: E7xxx

use thiserror::Error;

/// Terminal operation errors
#[derive(Error, Debug)]
pub enum TerminalError {
    #[error("[E7001] PTY creation failed: {reason}")]
    PtyCreationFailed { reason: String },

    #[error("[E7002] PTY I/O error: {details}")]
    PtyIoError { details: String },

    #[error("[E7003] Emulator error: {details}")]
    EmulatorError { details: String },

    #[error("[E7004] Scrollback error: {details}")]
    ScrollbackError { details: String },

    #[error("[E7005] IPC error: {details}")]
    IpcError { details: String },

    #[error("[E7006] Pane limit reached: max {max}")]
    PaneLimitReached { max: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ErrorCategory, GwtError};

    // --- TerminalError Display tests ---

    #[test]
    fn test_pty_creation_failed_display() {
        let err = TerminalError::PtyCreationFailed {
            reason: "no pty available".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("[E7001]"), "Expected E7001 in: {msg}");
        assert!(
            msg.contains("no pty available"),
            "Expected reason in: {msg}"
        );
    }

    #[test]
    fn test_pty_io_error_display() {
        let err = TerminalError::PtyIoError {
            details: "read failed".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("[E7002]"), "Expected E7002 in: {msg}");
        assert!(msg.contains("read failed"), "Expected details in: {msg}");
    }

    #[test]
    fn test_emulator_error_display() {
        let err = TerminalError::EmulatorError {
            details: "invalid escape".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("[E7003]"), "Expected E7003 in: {msg}");
        assert!(msg.contains("invalid escape"), "Expected details in: {msg}");
    }

    #[test]
    fn test_scrollback_error_display() {
        let err = TerminalError::ScrollbackError {
            details: "file corrupt".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("[E7004]"), "Expected E7004 in: {msg}");
        assert!(msg.contains("file corrupt"), "Expected details in: {msg}");
    }

    #[test]
    fn test_ipc_error_display() {
        let err = TerminalError::IpcError {
            details: "channel closed".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("[E7005]"), "Expected E7005 in: {msg}");
        assert!(msg.contains("channel closed"), "Expected details in: {msg}");
    }

    #[test]
    fn test_pane_limit_reached_display() {
        let err = TerminalError::PaneLimitReached { max: 4 };
        let msg = err.to_string();
        assert!(msg.contains("[E7006]"), "Expected E7006 in: {msg}");
        assert!(msg.contains("4"), "Expected max value in: {msg}");
    }

    // --- TerminalError -> GwtError From conversion tests ---

    #[test]
    fn test_terminal_error_into_gwt_error() {
        let terminal_err = TerminalError::PtyCreationFailed {
            reason: "test".to_string(),
        };
        let gwt_err: GwtError = terminal_err.into();
        assert!(
            matches!(gwt_err, GwtError::Terminal(_)),
            "Expected GwtError::Terminal variant"
        );
    }

    // --- GwtError::Terminal code() tests ---

    #[test]
    fn test_gwt_error_terminal_code_e7001() {
        let err: GwtError = TerminalError::PtyCreationFailed {
            reason: "test".to_string(),
        }
        .into();
        assert_eq!(err.code(), "E7001");
    }

    #[test]
    fn test_gwt_error_terminal_code_e7002() {
        let err: GwtError = TerminalError::PtyIoError {
            details: "test".to_string(),
        }
        .into();
        assert_eq!(err.code(), "E7002");
    }

    #[test]
    fn test_gwt_error_terminal_code_e7003() {
        let err: GwtError = TerminalError::EmulatorError {
            details: "test".to_string(),
        }
        .into();
        assert_eq!(err.code(), "E7003");
    }

    #[test]
    fn test_gwt_error_terminal_code_e7004() {
        let err: GwtError = TerminalError::ScrollbackError {
            details: "test".to_string(),
        }
        .into();
        assert_eq!(err.code(), "E7004");
    }

    #[test]
    fn test_gwt_error_terminal_code_e7005() {
        let err: GwtError = TerminalError::IpcError {
            details: "test".to_string(),
        }
        .into();
        assert_eq!(err.code(), "E7005");
    }

    #[test]
    fn test_gwt_error_terminal_code_e7006() {
        let err: GwtError = TerminalError::PaneLimitReached { max: 8 }.into();
        assert_eq!(err.code(), "E7006");
    }

    // --- GwtError::Terminal category() test ---

    #[test]
    fn test_gwt_error_terminal_category() {
        let err: GwtError = TerminalError::PtyCreationFailed {
            reason: "test".to_string(),
        }
        .into();
        assert_eq!(err.category(), ErrorCategory::Terminal);
    }

    #[test]
    fn test_gwt_error_terminal_category_all_variants() {
        let variants: Vec<GwtError> = vec![
            TerminalError::PtyCreationFailed {
                reason: "t".to_string(),
            }
            .into(),
            TerminalError::PtyIoError {
                details: "t".to_string(),
            }
            .into(),
            TerminalError::EmulatorError {
                details: "t".to_string(),
            }
            .into(),
            TerminalError::ScrollbackError {
                details: "t".to_string(),
            }
            .into(),
            TerminalError::IpcError {
                details: "t".to_string(),
            }
            .into(),
            TerminalError::PaneLimitReached { max: 1 }.into(),
        ];
        for err in variants {
            assert_eq!(
                err.category(),
                ErrorCategory::Terminal,
                "Expected Terminal category for {}",
                err
            );
        }
    }
}
