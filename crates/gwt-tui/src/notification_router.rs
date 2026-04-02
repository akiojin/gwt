//! Notification router — routes notifications by severity.

use crate::message::Message;

/// Notification severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Debug: log only, no UI.
    Debug,
    /// Info: status bar for 5 seconds.
    Info,
    /// Warning: status bar, persisted until dismissed.
    Warn,
    /// Error: modal overlay.
    Error,
}

/// A notification to be routed.
#[derive(Debug, Clone)]
pub struct Notification {
    pub severity: Severity,
    pub message: String,
}

/// Route a notification to the appropriate UI message.
///
/// Returns `Some(Message)` for notifications that need UI action,
/// `None` for debug-level (log-only).
pub fn route(notification: &Notification) -> Option<Message> {
    match notification.severity {
        Severity::Debug => {
            tracing::debug!("{}", notification.message);
            None
        }
        Severity::Info => {
            tracing::info!("{}", notification.message);
            // Phase 2: status bar notification with 5s timeout
            None
        }
        Severity::Warn => {
            tracing::warn!("{}", notification.message);
            // Phase 2: persistent status bar notification
            None
        }
        Severity::Error => {
            tracing::error!("{}", notification.message);
            Some(Message::PushError(notification.message.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_returns_none() {
        let n = Notification {
            severity: Severity::Debug,
            message: "debug msg".into(),
        };
        assert!(route(&n).is_none());
    }

    #[test]
    fn info_returns_none_for_now() {
        let n = Notification {
            severity: Severity::Info,
            message: "info msg".into(),
        };
        assert!(route(&n).is_none());
    }

    #[test]
    fn error_returns_push_error() {
        let n = Notification {
            severity: Severity::Error,
            message: "boom".into(),
        };
        let msg = route(&n);
        assert!(matches!(msg, Some(Message::PushError(ref s)) if s == "boom"));
    }

    #[test]
    fn warn_returns_none_for_now() {
        let n = Notification {
            severity: Severity::Warn,
            message: "warn msg".into(),
        };
        assert!(route(&n).is_none());
    }
}
