//! Error types for gwt-core.

/// A failed persisted Workspace read is distinct from an absent state file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStateLoadErrorKind {
    Io,
    Malformed,
    IncompatibleSchema,
    LegacyLayout,
}

/// Serializable diagnostic retained by the GUI while writes remain disabled.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
#[error("Workspace state load failed at {path}: {message}")]
pub struct WorkspaceStateLoadError {
    pub path: std::path::PathBuf,
    pub kind: WorkspaceStateLoadErrorKind,
    pub message: String,
}

impl WorkspaceStateLoadError {
    pub fn io(path: &std::path::Path, error: std::io::Error) -> Self {
        Self {
            path: path.to_path_buf(),
            kind: WorkspaceStateLoadErrorKind::Io,
            message: error.to_string(),
        }
    }

    pub fn json(path: &std::path::Path, error: serde_json::Error) -> Self {
        let kind = match error.classify() {
            serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
                WorkspaceStateLoadErrorKind::Malformed
            }
            serde_json::error::Category::Data => WorkspaceStateLoadErrorKind::IncompatibleSchema,
            serde_json::error::Category::Io => WorkspaceStateLoadErrorKind::Io,
        };
        Self {
            path: path.to_path_buf(),
            kind,
            message: error.to_string(),
        }
    }
}

/// Whether JSON cannot be parsed at all or is valid JSON produced by an
/// incompatible schema. Recovery may replace only malformed data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonDecodeKind {
    Malformed,
    IncompatibleSchema,
}

impl std::fmt::Display for JsonDecodeKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "malformed",
            Self::IncompatibleSchema => "incompatible schema",
        })
    }
}

/// Unified error type for all gwt operations.
#[derive(Debug, thiserror::Error)]
pub enum GwtError {
    /// Existing Workspace state could not be read safely.
    #[error(transparent)]
    WorkspaceStateLoad(#[from] WorkspaceStateLoadError),
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

    /// JSON decode error with a recovery-safe classification.
    #[error("{context} ({kind}): {message}")]
    JsonDecode {
        context: &'static str,
        kind: JsonDecodeKind,
        message: String,
    },

    /// An externally coordinated Workspace operation was resolved through a
    /// different current/WorkItems storage pair than the one it prepared.
    #[error(
        "external workspace operation {operation_id} is bound to a different current/work-items path pair"
    )]
    ExternalWorkspacePathPairMismatch { operation_id: String },

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
