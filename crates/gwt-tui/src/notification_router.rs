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
            Some(Message::ShowNotification(notification.clone()))
        }
        Severity::Warn => {
            tracing::warn!("{}", notification.message);
            Some(Message::ShowNotification(notification.clone()))
        }
        Severity::Error => {
            tracing::error!("{}", notification.message);
            Some(Message::PushErrorNotification(notification.clone()))
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
    fn info_returns_status_notification() {
        let n = Notification::new(Severity::Info, "router", "info msg");
        let msg = route(&n);
        assert!(
            matches!(msg, Some(Message::ShowNotification(ref notification))
            if notification.severity == Severity::Info
            && notification.source == "router"
            && notification.message == "info msg")
        );
    }

    #[test]
    fn error_returns_push_error() {
        let n = Notification::new(Severity::Error, "router", "boom").with_detail("stack trace");
        let msg = route(&n);
        assert!(
            matches!(msg, Some(Message::PushErrorNotification(ref notification))
            if notification.severity == Severity::Error
            && notification.source == "router"
            && notification.message == "boom"
            && notification.detail.as_deref() == Some("stack trace"))
        );
    }

    #[test]
    fn warn_returns_status_notification() {
        let n = Notification::new(Severity::Warn, "router", "warn msg");
        let msg = route(&n);
        assert!(
            matches!(msg, Some(Message::ShowNotification(ref notification))
            if notification.severity == Severity::Warn
            && notification.source == "router"
            && notification.message == "warn msg")
        );
    }
}
