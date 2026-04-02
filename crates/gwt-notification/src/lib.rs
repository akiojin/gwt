//! gwt-notification: Notification bus and structured logging for gwt
//!
//! Provides a bounded async notification channel and a ring-buffered
//! structured log with severity-based filtering (SPEC-6).

mod bus;
mod log;
mod notification;
mod severity;

pub use bus::{NotificationBus, NotificationReceiver};
pub use log::StructuredLog;
pub use notification::Notification;
pub use severity::Severity;
