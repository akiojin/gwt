//! Canonical log file reader (SPEC-1924 US-14 / FR-035 / FR-036 / FR-037).
//!
//! The structured runtime log file at
//! `~/.gwt/projects/<repo-hash>/logs/gwt.log.YYYY-MM-DD` is written by
//! `tracing_subscriber::fmt::layer().json()` (see `fmt_layer.rs`) using the
//! shape:
//!
//! ```json
//! {"timestamp":"...","level":"INFO","fields":{"message":"...","k":"v"},"target":"..."}
//! ```
//!
//! In-process the UI forwarder produces [`LogEvent`] values with an
//! auto-assigned `id`, mapped `severity` / `source`, and a hoisted `message`.
//! These two shapes are intentionally different: the on-disk shape is the
//! standard `tracing` JSON Lines format (compatible with external tooling)
//! and `id` is only meaningful inside a single process.
//!
//! This module provides the glue: [`LogFileEntry`] mirrors the on-disk shape
//! exactly, and [`read_log_file`] turns the JSONL file into a
//! [`Vec<LogEvent>`] suitable for the Logs window snapshot path. Malformed
//! lines are skipped and counted in [`ReadDiagnostics`] so a single corrupt
//! flush never blanks the snapshot.
//!
//! Logs window, future CLI subcommands, and the daemon must all go through
//! this reader; do not re-implement `serde_json::from_str::<LogEvent>` on
//! disk lines (SPEC-1924 FR-037 / SC-011).
//!
//! See [`mod@tracing_subscriber::fmt`] for the writer side; the relevant
//! configuration lives in `crates/gwt-core/src/logging/fmt_layer.rs`.

use std::{
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{LogEvent, LogLevel};

/// Result of [`read_log_file`].
#[derive(Debug, Clone)]
pub struct ReadOutcome {
    /// Successfully decoded log events in source order. `id` is assigned in
    /// read order starting at `1`.
    pub entries: Vec<LogEvent>,
    /// Diagnostic counters describing the read (skipped malformed lines etc.).
    pub diagnostics: ReadDiagnostics,
}

/// Diagnostic counters for a single [`read_log_file`] invocation.
#[derive(Debug, Clone)]
pub struct ReadDiagnostics {
    /// Absolute path of the file that was read (so the UI can surface the
    /// source when warning about skipped lines).
    pub path: PathBuf,
    /// Number of non-empty lines that failed JSON / shape validation and were
    /// skipped. Empty lines do not count.
    pub skipped: usize,
}

/// JSON Lines record as emitted by `tracing_subscriber::fmt::json()`.
///
/// Top-level fields not listed here (`span`, `spans`, `threadId`, …) are
/// silently ignored via `#[serde(deny_unknown_fields)]` *not* being set.
#[derive(Debug, Clone, Deserialize)]
pub struct LogFileEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

impl LogFileEntry {
    /// Convert this on-disk record into an in-memory [`LogEvent`].
    ///
    /// `id` is taken from the caller so that [`read_log_file`] can assign
    /// monotonically increasing ids in source order without any global state.
    pub fn into_log_event(mut self, id: u64) -> LogEvent {
        let severity = parse_level(&self.level);

        let message = take_string(&mut self.fields, "message").unwrap_or_default();
        let detail = take_string(&mut self.fields, "detail");

        LogEvent {
            id,
            severity,
            source: self.target,
            message,
            detail,
            timestamp: self.timestamp,
            fields: self.fields,
        }
    }
}

/// Read a canonical structured log file and return the decoded events.
///
/// Behavior:
///
/// - Returns `Ok(ReadOutcome { entries: vec![], diagnostics: { skipped: 0 } })`
///   when `path` does not exist (matches the previous GUI behavior: an empty
///   Logs window is acceptable before any event has been emitted).
/// - Empty lines and an unterminated trailing line are tolerated and do not
///   count as `skipped`.
/// - Lines that fail to decode are skipped and counted in
///   `diagnostics.skipped`; the rest are returned.
/// - `id` is assigned in read order starting from `1` and is in-memory only.
pub fn read_log_file(path: &Path) -> io::Result<ReadOutcome> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ReadOutcome {
                entries: Vec::new(),
                diagnostics: ReadDiagnostics {
                    path: path.to_path_buf(),
                    skipped: 0,
                },
            });
        }
        Err(error) => return Err(error),
    };

    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    let mut skipped = 0_usize;
    let mut next_id: u64 = 1;

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<LogFileEntry>(trimmed) {
            Ok(entry) => {
                entries.push(entry.into_log_event(next_id));
                next_id += 1;
            }
            Err(_) => {
                skipped += 1;
            }
        }
    }

    Ok(ReadOutcome {
        entries,
        diagnostics: ReadDiagnostics {
            path: path.to_path_buf(),
            skipped,
        },
    })
}

fn parse_level(level: &str) -> LogLevel {
    match level.to_ascii_uppercase().as_str() {
        "ERROR" => LogLevel::Error,
        "WARN" | "WARNING" => LogLevel::Warn,
        "INFO" => LogLevel::Info,
        // tracing emits DEBUG/TRACE — both collapse to Debug to match
        // `LogLevel::from_tracing`.
        _ => LogLevel::Debug,
    }
}

fn take_string(
    fields: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    match fields.remove(key)? {
        serde_json::Value::String(s) => Some(s),
        // tracing sometimes records `Debug` fields as their `{:?}` string
        // form which serializes as a JSON string anyway; everything else
        // (numbers, booleans, objects) is unexpected for `message`/`detail`
        // and we fall back to its JSON representation rather than dropping
        // the information silently.
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;

    use super::*;

    fn write_lines(lines: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("gwt.log.2026-05-20");
        let mut file = std::fs::File::create(&path).expect("create");
        for line in lines {
            file.write_all(line.as_bytes()).expect("write line");
            file.write_all(b"\n").expect("write newline");
        }
        (dir, path)
    }

    const PROD_LINE_INFO: &str = r#"{"timestamp":"2026-05-20T09:00:00.015355+09:00","level":"INFO","fields":{"message":"PTY resize completed","cols":72,"rows":24,"outcome":"ok"},"target":"gwt::resize::pty"}"#;
    const PROD_LINE_WARN: &str = r#"{"timestamp":"2026-05-20T09:00:00.020000+09:00","level":"WARN","fields":{"message":"slow flush","elapsed_ms":120},"target":"gwt::flush"}"#;
    const PROD_LINE_ERROR: &str = r#"{"timestamp":"2026-05-20T09:00:00.030000+09:00","level":"ERROR","fields":{"message":"git failed","detail":"exit status 128"},"target":"gwt::git"}"#;
    const PROD_LINE_NO_MESSAGE: &str = r#"{"timestamp":"2026-05-20T09:00:00.040000+09:00","level":"INFO","fields":{"k":"v"},"target":"gwt::nomsg"}"#;
    const UNKNOWN_FIELD_LINE: &str = r#"{"timestamp":"2026-05-20T09:00:00.050000+09:00","level":"INFO","fields":{"message":"with span"},"target":"gwt::span","span":{"name":"outer"},"spans":[{"name":"outer"}]}"#;
    const MALFORMED_LINE: &str = r#"{"foo":"bar"}"#;

    #[test]
    fn reads_production_shape_jsonl_round_trip() {
        let (_dir, path) = write_lines(&[PROD_LINE_INFO, PROD_LINE_WARN, PROD_LINE_ERROR]);

        let outcome = read_log_file(&path).expect("read ok");

        assert_eq!(outcome.diagnostics.skipped, 0);
        assert_eq!(outcome.entries.len(), 3);

        // ids are assigned in source order, in-memory only.
        assert_eq!(outcome.entries[0].id, 1);
        assert_eq!(outcome.entries[1].id, 2);
        assert_eq!(outcome.entries[2].id, 3);

        // level -> severity
        assert_eq!(outcome.entries[0].severity, LogLevel::Info);
        assert_eq!(outcome.entries[1].severity, LogLevel::Warn);
        assert_eq!(outcome.entries[2].severity, LogLevel::Error);

        // target -> source, fields.message -> message
        assert_eq!(outcome.entries[0].source, "gwt::resize::pty");
        assert_eq!(outcome.entries[0].message, "PTY resize completed");

        // fields.detail -> detail
        assert_eq!(
            outcome.entries[2].detail.as_deref(),
            Some("exit status 128")
        );

        // residual fields retained, message/detail removed.
        assert!(!outcome.entries[0].fields.contains_key("message"));
        assert_eq!(
            outcome.entries[0].fields.get("outcome"),
            Some(&serde_json::Value::String("ok".to_string()))
        );
        assert!(!outcome.entries[2].fields.contains_key("detail"));
    }

    #[test]
    fn skips_malformed_lines_and_returns_remaining_entries() {
        let (_dir, path) = write_lines(&[PROD_LINE_INFO, MALFORMED_LINE, PROD_LINE_WARN]);

        let outcome = read_log_file(&path).expect("read ok");

        assert_eq!(outcome.entries.len(), 2);
        assert_eq!(outcome.diagnostics.skipped, 1);
        assert_eq!(outcome.diagnostics.path, path);
        // ids are assigned only to successfully decoded entries and remain
        // monotonic.
        assert_eq!(outcome.entries[0].id, 1);
        assert_eq!(outcome.entries[1].id, 2);
    }

    #[test]
    fn missing_file_returns_empty_outcome() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.log");

        let outcome = read_log_file(&path).expect("read ok");

        assert!(outcome.entries.is_empty());
        assert_eq!(outcome.diagnostics.skipped, 0);
        assert_eq!(outcome.diagnostics.path, path);
    }

    #[test]
    fn empty_file_returns_empty_outcome() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("gwt.log.empty");
        std::fs::File::create(&path).expect("create");

        let outcome = read_log_file(&path).expect("read ok");

        assert!(outcome.entries.is_empty());
        assert_eq!(outcome.diagnostics.skipped, 0);
    }

    #[test]
    fn ignores_blank_lines_without_counting_as_skipped() {
        let (_dir, path) = write_lines(&["", "   ", PROD_LINE_INFO, "", PROD_LINE_WARN, "   "]);

        let outcome = read_log_file(&path).expect("read ok");

        assert_eq!(outcome.entries.len(), 2);
        assert_eq!(outcome.diagnostics.skipped, 0);
    }

    #[test]
    fn tolerates_trailing_unterminated_line() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("gwt.log.trail");
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(PROD_LINE_INFO.as_bytes()).expect("write");
        file.write_all(b"\n").expect("write nl");
        // last line has no trailing newline
        file.write_all(PROD_LINE_WARN.as_bytes()).expect("write");

        let outcome = read_log_file(&path).expect("read ok");

        assert_eq!(outcome.entries.len(), 2);
        assert_eq!(outcome.diagnostics.skipped, 0);
    }

    #[test]
    fn unknown_top_level_fields_are_ignored() {
        let (_dir, path) = write_lines(&[UNKNOWN_FIELD_LINE]);

        let outcome = read_log_file(&path).expect("read ok");

        assert_eq!(outcome.entries.len(), 1);
        assert_eq!(outcome.diagnostics.skipped, 0);
        assert_eq!(outcome.entries[0].message, "with span");
        assert_eq!(outcome.entries[0].source, "gwt::span");
        // span / spans are not mapped to LogEvent and must not leak into
        // `fields` (those keys are siblings of `fields`, not children).
        assert!(!outcome.entries[0].fields.contains_key("span"));
        assert!(!outcome.entries[0].fields.contains_key("spans"));
    }

    #[test]
    fn missing_message_field_yields_empty_message() {
        let (_dir, path) = write_lines(&[PROD_LINE_NO_MESSAGE]);

        let outcome = read_log_file(&path).expect("read ok");

        assert_eq!(outcome.entries.len(), 1);
        assert_eq!(outcome.entries[0].message, "");
        assert_eq!(
            outcome.entries[0].fields.get("k"),
            Some(&serde_json::Value::String("v".to_string()))
        );
    }
}
