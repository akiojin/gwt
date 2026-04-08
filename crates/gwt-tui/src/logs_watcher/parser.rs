//! JSONL → `LogEvent` parser used by the Logs tab file watcher.

use chrono::{DateTime, Utc};
use gwt_core::logging::{LogEvent, LogLevel};
use serde_json::Value;

/// Parse a single JSONL line produced by `tracing_subscriber::fmt::json`
/// into a `LogEvent`.
///
/// Never fails: malformed lines become a synthetic `ERROR`-level event
/// so that the developer notices that the log file contains garbage
/// rather than silently dropping lines.
pub fn parse_line(raw: &str) -> LogEvent {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return LogEvent::new(LogLevel::Debug, "gwt_tui::logs_watcher", "<empty line>");
    }

    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return LogEvent::new(
            LogLevel::Error,
            "gwt_tui::logs_watcher",
            "failed to parse JSONL line",
        )
        .with_detail(trimmed.to_string());
    };

    let Value::Object(mut map) = value else {
        return LogEvent::new(
            LogLevel::Error,
            "gwt_tui::logs_watcher",
            "JSONL line is not an object",
        )
        .with_detail(trimmed.to_string());
    };

    let severity = map
        .remove("level")
        .and_then(|v| v.as_str().map(level_from_str))
        .unwrap_or(LogLevel::Info);

    let source = map
        .remove("target")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    let timestamp = map
        .remove("timestamp")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    // The fmt::json layer writes the event body under a `fields` object:
    //   {"timestamp":"…","level":"INFO","fields":{"message":"…","k":"v"},"target":"…"}
    // but older versions flatten the message at the top level. Handle
    // both shapes by peeling the `fields` sub-object if present.
    let mut fields = map;
    if let Some(Value::Object(inner)) = fields.remove("fields") {
        for (k, v) in inner {
            fields.insert(k, v);
        }
    }

    // Pull `message` and `detail` out of the remaining fields so they
    // don't show up twice in the UI (once as the message, once in
    // the kv grid).
    let message = fields
        .remove("message")
        .map(|v| match v {
            Value::String(s) => s,
            other => other.to_string(),
        })
        .unwrap_or_default();

    let detail = fields.remove("detail").and_then(|v| match v {
        Value::String(s) => Some(s),
        Value::Null => None,
        other => Some(other.to_string()),
    });

    // Strip internal keys that fmt::json emits but the UI should not show.
    for key in ["span", "spans", "threadId", "threadName", "file", "line"] {
        fields.remove(key);
    }

    let mut event = LogEvent {
        id: 0,
        severity,
        source,
        message,
        detail,
        timestamp,
        fields,
    };
    // Re-use the auto-id counter on the LogEvent so the file-sourced
    // events get monotonic ids alongside in-process events. `LogEvent::new`
    // bumps the counter internally, so call it once to get a fresh id
    // and then replace the fields we care about.
    let placeholder = LogEvent::new(LogLevel::Debug, "", "");
    event.id = placeholder.id;
    event
}

fn level_from_str(s: &str) -> LogLevel {
    match s.to_ascii_uppercase().as_str() {
        "ERROR" => LogLevel::Error,
        "WARN" | "WARNING" => LogLevel::Warn,
        "INFO" => LogLevel::Info,
        "DEBUG" | "TRACE" => LogLevel::Debug,
        _ => LogLevel::Info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_well_formed_info_line() {
        let raw = r#"{"timestamp":"2026-04-08T10:00:00+09:00","level":"INFO","target":"gwt_tui::main","fields":{"message":"hello","session_id":"abc-123"}}"#;
        let e = parse_line(raw);
        assert_eq!(e.severity, LogLevel::Info);
        assert_eq!(e.source, "gwt_tui::main");
        assert_eq!(e.message, "hello");
        assert_eq!(
            e.fields.get("session_id"),
            Some(&Value::String("abc-123".into()))
        );
    }

    #[test]
    fn parses_error_with_detail_field() {
        let raw = r#"{"timestamp":"2026-04-08T10:00:00+09:00","level":"ERROR","target":"gwt_tui::panic","fields":{"message":"panic","detail":"stack trace"}}"#;
        let e = parse_line(raw);
        assert_eq!(e.severity, LogLevel::Error);
        assert_eq!(e.detail.as_deref(), Some("stack trace"));
    }

    #[test]
    fn malformed_json_becomes_synthetic_error_event() {
        let raw = r#"{not valid json"#;
        let e = parse_line(raw);
        assert_eq!(e.severity, LogLevel::Error);
        assert_eq!(e.source, "gwt_tui::logs_watcher");
        assert!(e.detail.as_deref().unwrap_or("").contains("not valid json"));
    }

    #[test]
    fn empty_line_becomes_debug_placeholder() {
        let e = parse_line("");
        assert_eq!(e.severity, LogLevel::Debug);
    }

    #[test]
    fn flat_message_without_fields_object() {
        let raw = r#"{"timestamp":"2026-04-08T10:00:00+09:00","level":"WARN","target":"t","message":"flat"}"#;
        let e = parse_line(raw);
        assert_eq!(e.severity, LogLevel::Warn);
        assert_eq!(e.message, "flat");
    }

    #[test]
    fn unknown_level_defaults_to_info() {
        let raw = r#"{"timestamp":"2026-04-08T10:00:00+09:00","level":"???","target":"t","fields":{"message":"x"}}"#;
        let e = parse_line(raw);
        assert_eq!(e.severity, LogLevel::Info);
    }
}
