//! Notification router — routes notifications by severity.

use crate::message::Message;
use gwt_core::logging::{LogEvent as Notification, LogLevel as Severity};

/// Route a notification to the appropriate UI message.
///
/// Returns `Some(Message)` for notifications that need UI action,
/// `None` for debug-level (log-only).
///
/// All severities emit a matching `tracing::*!` event so the
/// structured log file (`~/.gwt/logs/gwt.log.YYYY-MM-DD`) and the
/// Logs tab (via the file watcher) pick up the event. The UI message
/// path remains for toast / error modal surfaces until the tracing
/// UI forwarder Layer is fully wired in Step 3.
pub fn route(notification: &Notification) -> Option<Message> {
    let detail = notification.detail.as_deref().unwrap_or("");
    match notification.severity {
        Severity::Debug => {
            tracing::debug!(
                target: "gwt_tui::notify",
                source = %notification.source,
                detail = %detail,
                "{}",
                notification.message
            );
            None
        }
        Severity::Info => {
            tracing::info!(
                target: "gwt_tui::notify",
                source = %notification.source,
                detail = %detail,
                "{}",
                notification.message
            );
            Some(Message::ShowNotification(notification.clone()))
        }
        Severity::Warn => {
            tracing::warn!(
                target: "gwt_tui::notify",
                source = %notification.source,
                detail = %detail,
                "{}",
                notification.message
            );
            Some(Message::ShowNotification(notification.clone()))
        }
        Severity::Error => {
            tracing::error!(
                target: "gwt_tui::notify",
                source = %notification.source,
                detail = %detail,
                "{}",
                notification.message
            );
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
