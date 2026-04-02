use crate::{Notification, Severity};

/// Maximum entries in the ring buffer.
const MAX_ENTRIES: usize = 10_000;

/// Ring-buffered structured log of notifications.
#[derive(Debug, Clone)]
pub struct StructuredLog {
    entries: Vec<Notification>,
    /// Write position in the ring buffer.
    head: usize,
    /// Total entries written (may exceed MAX_ENTRIES).
    len: usize,
}

impl StructuredLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::with_capacity(256),
            head: 0,
            len: 0,
        }
    }

    /// Push a notification into the ring buffer.
    pub fn push(&mut self, notification: Notification) {
        if self.entries.len() < MAX_ENTRIES {
            self.entries.push(notification);
        } else {
            self.entries[self.head] = notification;
        }
        self.head = (self.head + 1) % MAX_ENTRIES;
        self.len += 1;
    }

    /// Return entries in insertion order.
    pub fn entries(&self) -> Vec<&Notification> {
        let actual_len = self.entries.len();
        if actual_len < MAX_ENTRIES || self.len <= MAX_ENTRIES {
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
}
