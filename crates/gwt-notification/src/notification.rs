use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::Severity;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A single notification entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: u64,
    pub severity: Severity,
    pub source: String,
    pub message: String,
    pub detail: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl Notification {
    /// Create a new notification with auto-assigned id and current timestamp.
    pub fn new(severity: Severity, source: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            severity,
            source: source.into(),
            message: message.into(),
            detail: None,
            timestamp: Utc::now(),
        }
    }

    /// Attach optional detail text.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_assigns_unique_ids() {
        let a = Notification::new(Severity::Info, "test", "msg-a");
        let b = Notification::new(Severity::Info, "test", "msg-b");
        assert_ne!(a.id, b.id);
        assert!(b.id > a.id);
    }

    #[test]
    fn new_sets_fields() {
        let n = Notification::new(Severity::Warn, "git", "conflict detected");
        assert_eq!(n.severity, Severity::Warn);
        assert_eq!(n.source, "git");
        assert_eq!(n.message, "conflict detected");
        assert!(n.detail.is_none());
    }

    #[test]
    fn with_detail_sets_detail() {
        let n = Notification::new(Severity::Error, "pty", "crash")
            .with_detail("segfault at 0x0");
        assert_eq!(n.detail.as_deref(), Some("segfault at 0x0"));
    }

    #[test]
    fn timestamp_is_recent() {
        let before = Utc::now();
        let n = Notification::new(Severity::Debug, "test", "ts check");
        let after = Utc::now();
        assert!(n.timestamp >= before && n.timestamp <= after);
    }
}
