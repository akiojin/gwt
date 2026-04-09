//! Error types for gwt-core.

/// Unified error type for all gwt operations.
#[derive(Debug, thiserror::Error)]
pub enum GwtError {
    /// I/O error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Git operation error.
    #[error("Git error: {0}")]
    Git(String),

    /// Configuration error.
    #[error("Config error: {0}")]
    Config(String),

    /// Agent launch/communication error.
    #[error("Agent error: {0}")]
    Agent(String),

    /// Terminal/PTY error.
    #[error("Terminal error: {0}")]
    Terminal(String),

    /// Docker operation error.
    #[error("Docker error: {0}")]
    Docker(String),

    /// AI provider error.
    #[error("AI error: {0}")]
    Ai(String),

    /// Notification error.
    #[error("Notification error: {0}")]
    Notification(String),

    /// Voice input/output error.
    #[error("Voice error: {0}")]
    Voice(String),

    /// Clipboard error.
    #[error("Clipboard error: {0}")]
    Clipboard(String),

    /// Skill execution error.
    #[error("Skill error: {0}")]
    Skill(String),

    /// Catch-all for uncategorised errors.
    #[error("{0}")]
    Other(String),
}

/// Convenience alias used throughout the crate and dependents.
pub type Result<T> = std::result::Result<T, GwtError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_converts_from_std() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let gwt_err: GwtError = io_err.into();
        assert!(matches!(gwt_err, GwtError::Io(_)));
        assert!(gwt_err.to_string().contains("gone"));
    }

    #[test]
    fn git_error_displays_message() {
        let err = GwtError::Git("bad ref".into());
        assert_eq!(err.to_string(), "Git error: bad ref");
    }

    #[test]
    fn config_error_displays_message() {
        let err = GwtError::Config("missing key".into());
        assert_eq!(err.to_string(), "Config error: missing key");
    }

    #[test]
    fn agent_error_displays_message() {
        let err = GwtError::Agent("timeout".into());
        assert_eq!(err.to_string(), "Agent error: timeout");
    }

    #[test]
    fn terminal_error_displays_message() {
        let err = GwtError::Terminal("pty failed".into());
        assert_eq!(err.to_string(), "Terminal error: pty failed");
    }

    #[test]
    fn docker_error_displays_message() {
        let err = GwtError::Docker("daemon not running".into());
        assert_eq!(err.to_string(), "Docker error: daemon not running");
    }

    #[test]
    fn ai_error_displays_message() {
        let err = GwtError::Ai("rate limited".into());
        assert_eq!(err.to_string(), "AI error: rate limited");
    }

    #[test]
    fn notification_error_displays_message() {
        let err = GwtError::Notification("send failed".into());
        assert_eq!(err.to_string(), "Notification error: send failed");
    }

    #[test]
    fn voice_error_displays_message() {
        let err = GwtError::Voice("mic unavailable".into());
        assert_eq!(err.to_string(), "Voice error: mic unavailable");
    }

    #[test]
    fn clipboard_error_displays_message() {
        let err = GwtError::Clipboard("paste failed".into());
        assert_eq!(err.to_string(), "Clipboard error: paste failed");
    }

    #[test]
    fn skill_error_displays_message() {
        let err = GwtError::Skill("not found".into());
        assert_eq!(err.to_string(), "Skill error: not found");
    }

    #[test]
    fn other_error_displays_raw_message() {
        let err = GwtError::Other("something unexpected".into());
        assert_eq!(err.to_string(), "something unexpected");
    }

    #[test]
    fn gwt_error_is_std_error() {
        let err: Box<dyn std::error::Error> = Box::new(GwtError::Other("test".into()));
        assert!(err.to_string().contains("test"));
    }

    #[test]
    fn result_alias_works() {
        fn ok_fn() -> Result<i32> {
            Ok(42)
        }
        fn err_fn() -> Result<i32> {
            Err(GwtError::Other("nope".into()))
        }
        assert_eq!(ok_fn().unwrap(), 42);
        assert!(err_fn().is_err());
    }
}
