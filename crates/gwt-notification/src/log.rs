use crate::{Notification, Severity};

/// Maximum entries in the ring buffer.
const MAX_ENTRIES: usize = 10_000;

/// Ring-buffered structured log of notifications.
#[derive(Debug, Clone)]
pub struct StructuredLog {
    entries: Vec<Notification>,
    /// Maximum number of stored entries before wrapping.
    capacity: usize,
    /// Write position in the ring buffer.
    head: usize,
    /// Total entries written (may exceed capacity).
    len: usize,
}

impl StructuredLog {
    pub fn new() -> Self {
        Self::with_capacity(MAX_ENTRIES)
    }

    /// Create a log with a custom ring buffer capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            entries: Vec::with_capacity(capacity),
            capacity,
            head: 0,
            len: 0,
        }
    }

    /// Push a notification into the ring buffer.
    pub fn push(&mut self, notification: Notification) {
        if self.entries.len() < self.capacity {
            self.entries.push(notification);
        } else {
            self.entries[self.head] = notification;
        }
        self.head = (self.head + 1) % self.capacity;
        self.len += 1;
    }

    /// Return entries in insertion order.
    pub fn entries(&self) -> Vec<&Notification> {
        let actual_len = self.entries.len();
        if actual_len < self.capacity || self.len <= self.capacity {
            // Not yet wrapped
            self.entries.iter().collect()
        } else {
            // Wrapped: oldest is at head, read head..end then 0..head
            let mut result = Vec::with_capacity(actual_len);
            result.extend(self.entries[self.head..].iter());
            result.extend(self.entries[..self.head].iter());
            result
        }
    }

    /// Filter entries with severity >= the given threshold.
    pub fn filter(&self, min_severity: Severity) -> Vec<&Notification> {
        self.entries()
            .into_iter()
            .filter(|n| n.severity >= min_severity)
            .collect()
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.head = 0;
        self.len = 0;
    }

    /// Number of entries currently stored.
    pub fn count(&self) -> usize {
        self.entries.len()
    }
}

impl Default for StructuredLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(severity: Severity, msg: &str) -> Notification {
        Notification::new(severity, "test", msg)
    }

    #[test]
    fn push_and_entries() {
        let mut log = StructuredLog::new();
        log.push(make(Severity::Info, "a"));
        log.push(make(Severity::Warn, "b"));
        let entries = log.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "a");
        assert_eq!(entries[1].message, "b");
    }

    #[test]
    fn push_preserves_notification_fields() {
        let mut log = StructuredLog::new();
        let notification = Notification::new(Severity::Error, "git", "merge conflict");
        let expected = notification.clone();

        log.push(notification);

        let entry = log.entries()[0];
        assert_eq!(entry.id, expected.id);
        assert_eq!(entry.timestamp, expected.timestamp);
        assert_eq!(entry.severity, Severity::Error);
        assert_eq!(entry.source, "git");
        assert_eq!(entry.message, "merge conflict");
    }

    #[test]
    fn filter_by_severity() {
        let mut log = StructuredLog::new();
        log.push(make(Severity::Debug, "d"));
        log.push(make(Severity::Info, "i"));
        log.push(make(Severity::Warn, "w"));
        log.push(make(Severity::Error, "e"));

        let warn_plus = log.filter(Severity::Warn);
        assert_eq!(warn_plus.len(), 2);
        assert_eq!(warn_plus[0].message, "w");
        assert_eq!(warn_plus[1].message, "e");
    }

    #[test]
    fn filter_debug_returns_all() {
        let mut log = StructuredLog::new();
        log.push(make(Severity::Debug, "d"));
        log.push(make(Severity::Error, "e"));
        assert_eq!(log.filter(Severity::Debug).len(), 2);
    }

    #[test]
    fn clear_empties_log() {
        let mut log = StructuredLog::new();
        log.push(make(Severity::Info, "x"));
        log.push(make(Severity::Info, "y"));
        assert_eq!(log.count(), 2);
        log.clear();
        assert_eq!(log.count(), 0);
        assert!(log.entries().is_empty());
    }

    #[test]
    fn ring_buffer_wraps_at_max() {
        let mut log = StructuredLog::new();
        for i in 0..10_500 {
            log.push(make(Severity::Debug, &format!("msg-{i}")));
        }
        // Should cap at MAX_ENTRIES
        assert_eq!(log.count(), MAX_ENTRIES);

        let entries = log.entries();
        // Oldest surviving entry should be msg-500 (10500 - 10000)
        assert_eq!(entries[0].message, "msg-500");
        // Newest should be msg-10499
        assert_eq!(entries[MAX_ENTRIES - 1].message, "msg-10499");
    }

    #[test]
    fn default_is_empty() {
        let log = StructuredLog::default();
        assert_eq!(log.count(), 0);
        assert!(log.entries().is_empty());
    }

    #[test]
    fn with_capacity_controls_ring_size() {
        let mut log = StructuredLog::with_capacity(3);
        for i in 0..10 {
            log.push(make(Severity::Debug, &format!("msg-{i}")));
        }

        assert_eq!(log.count(), 3);
        let entries = log.entries();
        assert_eq!(entries[0].message, "msg-7");
        assert_eq!(entries[1].message, "msg-8");
        assert_eq!(entries[2].message, "msg-9");
    }

    #[test]
    fn new_uses_default_capacity() {
        let log = StructuredLog::new();
        assert_eq!(log.entries.capacity(), MAX_ENTRIES);
    }
}
