use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::Serialize;

use crate::UiTraceEntry;

const PERF_RECORD_SCHEMA_VERSION: u32 = 1;
const PERF_FIELD_MAX_CHARS: usize = 160;
const BLOCKED_FIELDS: &[&str] = &[
    "body",
    "chunk",
    "data",
    "data_base64",
    "input",
    "payload",
    "text",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PerfStream {
    Ui,
    Op,
    Resource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PerfUnit {
    #[serde(rename = "ms")]
    Milliseconds,
    Percent,
    Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PerfRecordType {
    Sample,
    Violation,
}

/// Evidence captured when a sustained budget violation is emitted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerfViolationDetails {
    budget: f64,
    consecutive_count: u32,
    duration_seconds: f64,
}

impl PerfViolationDetails {
    pub fn new(budget: f64, consecutive_count: u32, duration_seconds: f64) -> Self {
        Self {
            budget,
            consecutive_count,
            duration_seconds,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PerfRecord {
    schema_version: u32,
    #[serde(rename = "type")]
    record_type: PerfRecordType,
    timestamp: DateTime<Utc>,
    stream: PerfStream,
    target: String,
    value: f64,
    unit: PerfUnit,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    consecutive_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_seconds: Option<f64>,
}

impl PerfRecord {
    pub fn sample(
        timestamp: DateTime<Utc>,
        stream: PerfStream,
        target: impl AsRef<str>,
        value: f64,
        unit: PerfUnit,
    ) -> Self {
        Self {
            schema_version: PERF_RECORD_SCHEMA_VERSION,
            record_type: PerfRecordType::Sample,
            timestamp,
            stream,
            target: sanitize_perf_target(target.as_ref()),
            value,
            unit,
            role: None,
            budget: None,
            consecutive_count: None,
            duration_seconds: None,
        }
    }

    pub fn violation(
        timestamp: DateTime<Utc>,
        stream: PerfStream,
        target: impl AsRef<str>,
        value: f64,
        unit: PerfUnit,
        details: PerfViolationDetails,
    ) -> Self {
        Self {
            schema_version: PERF_RECORD_SCHEMA_VERSION,
            record_type: PerfRecordType::Violation,
            timestamp,
            stream,
            target: sanitize_perf_target(target.as_ref()),
            value,
            unit,
            role: None,
            budget: Some(details.budget),
            consecutive_count: Some(details.consecutive_count),
            duration_seconds: Some(details.duration_seconds),
        }
    }

    pub fn with_role(mut self, role: impl AsRef<str>) -> Self {
        self.role = Some(sanitize_perf_target(role.as_ref()));
        self
    }
}

/// Strip control characters and cap a user-action field at 160 characters.
///
/// This is the canonical implementation shared with the binary-only frontend
/// action logger.
#[doc(hidden)]
pub fn sanitize_ui_action_field(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(PERF_FIELD_MAX_CHARS)
        .collect()
}

/// Apply the UI Trace scalar-field allowlist and sensitive-field denylist.
///
/// This is public only so the binary-only UI Trace writer can share the same
/// implementation as the library-owned perf record model.
#[doc(hidden)]
pub fn sanitize_ui_trace_entry(entry: &UiTraceEntry) -> serde_json::Value {
    let Some(object) = entry.fields() else {
        return serde_json::json!({ "kind": "invalid_entry" });
    };
    let mut sanitized = serde_json::Map::new();
    for (key, value) in object {
        if BLOCKED_FIELDS.contains(&normalize_field_name(key).as_str()) {
            continue;
        }
        if value.is_null() || value.is_boolean() || value.is_number() || value.is_string() {
            sanitized.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(sanitized)
}

fn normalize_field_name(key: &str) -> String {
    key.chars()
        .fold(String::new(), |mut normalized, character| {
            if character.is_ascii_uppercase() {
                normalized.push('_');
                normalized.push(character.to_ascii_lowercase());
            } else {
                normalized.push(character);
            }
            normalized
        })
}

fn sanitize_perf_target(value: &str) -> String {
    static ABSOLUTE_PATH: OnceLock<Regex> = OnceLock::new();

    let bounded = sanitize_ui_action_field(value);
    ABSOLUTE_PATH
        .get_or_init(|| {
            Regex::new(r#"(?:[A-Za-z]:[\\/]|/)[^\s\"']*"#).expect("perf target absolute path regex")
        })
        .replace_all(&bounded, "[redacted-path]")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone as _, Utc};
    use serde_json::json;

    use super::*;

    fn timestamp() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 20, 12, 34, 56)
            .single()
            .expect("valid timestamp")
    }

    #[test]
    fn serializes_sample_and_violation_with_versioned_tagged_schema() {
        let sample = PerfRecord::sample(
            timestamp(),
            PerfStream::Ui,
            "pointer_latency",
            12.5,
            PerfUnit::Milliseconds,
        );
        let sample_json = serde_json::to_string(&sample).expect("serialize sample");
        assert!(
            sample_json.starts_with(r#"{"schema_version":1,"type":"sample","#),
            "schema_version must be the first serialized field: {sample_json}"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&sample_json).expect("parse sample"),
            json!({
                "schema_version": 1,
                "type": "sample",
                "timestamp": "2026-08-20T12:34:56Z",
                "stream": "ui",
                "target": "pointer_latency",
                "value": 12.5,
                "unit": "ms"
            })
        );

        let violation = PerfRecord::violation(
            timestamp(),
            PerfStream::Op,
            "gwtd:issue.view",
            175.0,
            PerfUnit::Milliseconds,
            PerfViolationDetails::new(100.0, 3, 2.25),
        );
        let violation_json = serde_json::to_string(&violation).expect("serialize violation");
        assert!(violation_json.starts_with(r#"{"schema_version":1,"type":"violation","#));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&violation_json).expect("parse violation"),
            json!({
                "schema_version": 1,
                "type": "violation",
                "timestamp": "2026-08-20T12:34:56Z",
                "stream": "op",
                "target": "gwtd:issue.view",
                "value": 175.0,
                "unit": "ms",
                "budget": 100.0,
                "consecutive_count": 3,
                "duration_seconds": 2.25
            })
        );
    }

    #[test]
    fn stream_serialization_is_limited_to_ui_op_and_resource() {
        let serialized = [PerfStream::Ui, PerfStream::Op, PerfStream::Resource]
            .into_iter()
            .map(|stream| serde_json::to_value(stream).expect("serialize stream"))
            .collect::<Vec<_>>();

        assert_eq!(
            serialized,
            vec![json!("ui"), json!("op"), json!("resource")]
        );
    }

    #[test]
    fn shared_trace_sanitizer_drops_blocked_fields() {
        let entry = serde_json::from_value::<crate::UiTraceEntry>(json!({
            "kind": "pointer_measure",
            "duration_ms": 12.5,
            "data_base64": "must-not-leak-data",
            "dataBase64": "must-not-leak-camel-data",
            "payload": "must-not-leak-payload",
            "input": "must-not-leak-input",
            "text": "must-not-leak-text"
        }))
        .expect("deserialize trace entry");

        let sanitized = sanitize_ui_trace_entry(&entry);
        let serialized = serde_json::to_string(&sanitized).expect("serialize sanitized entry");

        assert_eq!(
            sanitized,
            json!({ "kind": "pointer_measure", "duration_ms": 12.5 })
        );
        assert!(!serialized.contains("must-not-leak"));
        for blocked in ["data_base64", "dataBase64", "payload", "input", "text"] {
            assert!(
                !serialized.contains(blocked),
                "blocked field leaked: {blocked}"
            );
        }
    }

    #[test]
    fn record_target_redacts_absolute_paths_and_caps_fields_at_160_characters() {
        for target in [
            "frontend:/Users/alice/private/repository/render",
            r"frontend:C:\Users\alice\private\repository\render",
        ] {
            let record = PerfRecord::sample(
                timestamp(),
                PerfStream::Ui,
                target,
                1.0,
                PerfUnit::Milliseconds,
            );
            let serialized = serde_json::to_string(&record).expect("serialize sample");
            assert!(
                !serialized.contains("Users"),
                "absolute path leaked: {serialized}"
            );
            assert!(
                !serialized.contains("private"),
                "absolute path leaked: {serialized}"
            );
            assert!(serialized.contains("[redacted-path]"));
        }

        let record = PerfRecord::sample(
            timestamp(),
            PerfStream::Resource,
            format!("{}\nsecret", "x".repeat(200)),
            42.0,
            PerfUnit::Bytes,
        );
        let value = serde_json::to_value(record).expect("serialize sample");
        let target = value["target"].as_str().expect("target string");
        assert_eq!(target.chars().count(), 160);
        assert!(!target.chars().any(char::is_control));
        assert!(!target.contains("secret"));
    }
}
