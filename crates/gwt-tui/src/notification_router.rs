//! Notification router — routes notifications by severity.

use crate::message::Message;
use gwt_notification::{Notification, Severity};

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
        let n = Notification::new(Severity::Debug, "router", "debug msg");
        assert!(route(&n).is_none());
    }

    #[test]
    fn info_returns_none() {
        let n = Notification::new(Severity::Info, "router", "info msg");
        assert!(route(&n).is_none());
    }

    #[test]
    fn error_returns_push_error() {
        let n = Notification::new(Severity::Error, "router", "boom");
        let msg = route(&n);
        assert!(matches!(msg, Some(Message::PushError(ref s)) if s == "boom"));
    }

    #[test]
    fn warn_returns_none() {
        let n = Notification::new(Severity::Warn, "router", "warn msg");
        assert!(route(&n).is_none());
    }
}
