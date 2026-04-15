//! `LogEvent` — the in-process representation of a tracing event.
//!
//! `LogEvent` replaces the retired `gwt_notification::Notification` struct.
//! It carries the exact same fields that the UI needs (`level`, `target`,
//! `message`, `detail`, `timestamp`, `id`) plus an optional free-form
//! `fields` map so that structured kv pairs from `tracing` events can be
//! round-tripped through the pipeline.
//!
//! The UI forwarder layer produces `LogEvent`s directly from
//! `tracing::Event`s. The JSONL parser in `gwt` produces them from
//! file lines. Both code paths share this type so UI widgets do not need
//! to care where the event came from.

use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::LogLevel;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A single log event. Compatible with the old `Notification` API on purpose
/// so that the Phase 5 substitution is a mechanical rename for call sites.
///
/// Field names (`severity`, `source`) intentionally match the retired
/// `Notification` struct so that existing TUI widgets and tests do not
/// need to be touched beyond the type name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub id: u64,
    pub severity: LogLevel,
    pub source: String,
    pub message: String,
    pub detail: Option<String>,
    pub timestamp: DateTime<Utc>,
    /// Extra structured fields collected from the tracing event
    /// (`tracing::field::Visit`). Empty when produced by a plain
    /// `tracing::info!("msg")` call without kv pairs.
    #[serde(default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

impl LogEvent {
    /// Construct a new event with auto-assigned id and current UTC timestamp.
    ///
    /// API-compatible with `Notification::new` so that existing call sites
    /// can be updated by a single-step rename.
    pub fn new(severity: LogLevel, source: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            severity,
            source: source.into(),
            message: message.into(),
            detail: None,
            timestamp: Utc::now(),
            fields: serde_json::Map::new(),
        }
    }

    /// Attach optional detail text (used by toasts and the error modal).
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Attach a structured field.
    pub fn with_field(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.fields.insert(key.into(), value);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_assigns_unique_ids() {
        let a = LogEvent::new(LogLevel::Info, "test", "msg-a");
        let b = LogEvent::new(LogLevel::Info, "test", "msg-b");
        assert!(b.id > a.id);
    }

    #[test]
    fn new_sets_fields() {
        let e = LogEvent::new(LogLevel::Warn, "git", "conflict detected");
        assert_eq!(e.severity, LogLevel::Warn);
        assert_eq!(e.source, "git");
        assert_eq!(e.message, "conflict detected");
        assert!(e.detail.is_none());
        assert!(e.fields.is_empty());
    }

    #[test]
    fn with_detail_sets_detail() {
        let e = LogEvent::new(LogLevel::Error, "pty", "crash").with_detail("segfault at 0x0");
        assert_eq!(e.detail.as_deref(), Some("segfault at 0x0"));
    }

    #[test]
    fn with_field_collects_structured_data() {
        let e = LogEvent::new(LogLevel::Info, "agent", "launch")
            .with_field("session_id", serde_json::json!("abc-123"));
        assert_eq!(
            e.fields.get("session_id"),
            Some(&serde_json::json!("abc-123"))
        );
    }

    #[test]
    fn timestamp_is_recent() {
        let before = Utc::now();
        let e = LogEvent::new(LogLevel::Debug, "test", "ts check");
        let after = Utc::now();
        assert!(e.timestamp >= before && e.timestamp <= after);
    }
}
