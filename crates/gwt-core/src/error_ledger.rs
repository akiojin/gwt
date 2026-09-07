//! Host-wide append-only error ledger (Issue #3778).
//!
//! Records launch failures, hook failures, operation refusals, and daemon
//! faults as daily JSONL under `~/.gwt/logs/errors/`. `errors.list` reads
//! this ledger; a GUI toast is not a source of truth on its own.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const LEDGER_FILE_PREFIX: &str = "errors";

/// Error classes the PM can query without reconstructing pane output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    LaunchFailure,
    HookFailure,
    OperationRefusal,
    DaemonFault,
}

/// Optional locators so a ledger row can be triaged back to a launch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
}

/// One append-only error row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorRecord {
    pub schema_version: u32,
    pub id: String,
    pub recorded_at: DateTime<Utc>,
    pub kind: ErrorKind,
    pub message: String,
    #[serde(default)]
    pub target: ErrorTarget,
    /// Kind-specific structured detail (Issue #3541: hook failures carry
    /// `event`, `handler`, `exit_status`, `fail_open`). Values are sanitized
    /// like `message`; older rows without the field still deserialize.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, String>,
}

impl ErrorRecord {
    pub fn new(kind: ErrorKind, message: impl Into<String>, target: ErrorTarget) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            recorded_at: Utc::now(),
            kind,
            message: sanitize_error_message(&message.into()),
            target: sanitize_target(target),
            context: BTreeMap::new(),
        }
    }

    /// Attach sanitized structured detail to the row.
    pub fn with_context(mut self, context: BTreeMap<String, String>) -> Self {
        self.context = context
            .into_iter()
            .map(|(key, value)| (key, sanitize_error_message(&value)))
            .collect();
        self
    }
}

/// Append `record` to today's ledger file. Fail-open callers should use
/// [`record_fail_open`].
pub fn record(record: ErrorRecord) -> io::Result<ErrorRecord> {
    #[cfg(any(test, feature = "test-support"))]
    if crate::test_support::gwt_home_override().is_none() {
        return Ok(record);
    }
    append_record(&record)?;
    Ok(record)
}

/// Best-effort record that never fails the originating operation.
pub fn record_fail_open(kind: ErrorKind, message: impl Into<String>, target: ErrorTarget) {
    let record = ErrorRecord::new(kind, message, target);
    if let Err(error) = self::record(record) {
        tracing::warn!(error = %error, "error ledger append failed");
    }
}

/// Return ledger rows at or after `since`, oldest first.
pub fn list_since(since: Option<DateTime<Utc>>) -> io::Result<Vec<ErrorRecord>> {
    #[cfg(any(test, feature = "test-support"))]
    if crate::test_support::gwt_home_override().is_none() {
        return Ok(Vec::new());
    }
    read_ledger_files(since)
}

fn ledger_dir() -> PathBuf {
    crate::paths::gwt_error_ledger_dir()
}

fn ledger_path_for_date(date: NaiveDate) -> PathBuf {
    ledger_dir().join(format!("{LEDGER_FILE_PREFIX}.{date}.jsonl"))
}

/// Strip terminal escapes, redact credentials, and bound the length of a
/// message before it reaches the ledger or a user-visible error line.
pub fn sanitize_error_message(message: &str) -> String {
    const MAX_CHARS: usize = 1_000;
    let stripped = crate::process_console::strip_ansi::strip_ansi(message);
    let mut redacted = redact_secrets(&crate::process_console::redact::redact_line(&stripped));
    redacted = redacted
        .chars()
        .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\t')
        .collect();
    if redacted.chars().count() > MAX_CHARS {
        redacted = redacted.chars().take(MAX_CHARS).collect();
        redacted.push('…');
    }
    redacted
}

fn sanitize_target(mut target: ErrorTarget) -> ErrorTarget {
    if let Some(window_id) = target.window_id.as_mut() {
        *window_id = sanitize_error_message(window_id);
    }
    if let Some(session_id) = target.session_id.as_mut() {
        *session_id = sanitize_error_message(session_id);
    }
    if let Some(project_root) = target.project_root.as_mut() {
        *project_root = sanitize_error_message(project_root);
    }
    target
}

fn redact_secrets(message: &str) -> String {
    const SENSITIVE: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "GWT_HOOK_TOKEN",
        "HOOK_TOKEN",
        "Bearer ",
        "bearer ",
    ];
    let mut redacted = message.to_string();
    for needle in SENSITIVE {
        while let Some(index) = redacted.find(needle) {
            let after_key = index + needle.len();
            let rest = &redacted[after_key..];
            let prefix_skip = rest
                .chars()
                .take_while(|ch| matches!(*ch, '=' | ':' | ' '))
                .count();
            let value = &rest[prefix_skip..];
            let value_len = value
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '-' | '_' | '.'))
                .count();
            let end = after_key + prefix_skip + value_len;
            redacted.replace_range(index..end, "[REDACTED]");
        }
    }
    redacted
}

fn append_record(record: &ErrorRecord) -> io::Result<()> {
    let dir = ledger_dir();
    fs::create_dir_all(&dir)?;
    let path = ledger_path_for_date(record.recorded_at.date_naive());
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, record).map_err(json_io_error)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn read_ledger_files(since: Option<DateTime<Utc>>) -> io::Result<Vec<ErrorRecord>> {
    let dir = ledger_dir();
    let mut paths = match fs::read_dir(&dir) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with(&format!("{LEDGER_FILE_PREFIX}."))
                            && name.ends_with(".jsonl")
                    })
            })
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    paths.sort();
    let mut records = Vec::new();
    for path in paths {
        let file = fs::File::open(path)?;
        for line in BufReader::new(file).lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(record) = serde_json::from_str::<ErrorRecord>(trimmed) else {
                continue;
            };
            if since.is_none_or(|since| record.recorded_at >= since) {
                records.push(record);
            }
        }
    }
    records.sort_by_key(|record| record.recorded_at);
    Ok(records)
}

fn json_io_error(error: serde_json::Error) -> io::Error {
    io::Error::other(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedGwtHome;
    use chrono::TimeZone;

    fn isolated_home() -> (tempfile::TempDir, ScopedGwtHome) {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = ScopedGwtHome::set(dir.path().join("gwt-home"));
        (dir, home)
    }

    fn sample(kind: ErrorKind, message: &str) -> ErrorRecord {
        ErrorRecord::new(
            kind,
            message,
            ErrorTarget {
                issue: Some(3778),
                window_id: Some("win-1".into()),
                session_id: Some("sess-1".into()),
                project_root: Some("/tmp/repo".into()),
            },
        )
    }

    #[test]
    fn record_persists_kind_message_and_target_for_list_since() {
        let (_dir, _home) = isolated_home();
        let recorded = record(sample(
            ErrorKind::LaunchFailure,
            "stale generation launch failed",
        ))
        .expect("record");

        let listed = list_since(None).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, recorded.id);
        assert_eq!(listed[0].kind, ErrorKind::LaunchFailure);
        assert_eq!(listed[0].message, "stale generation launch failed");
        assert_eq!(listed[0].target.issue, Some(3778));
        assert_eq!(listed[0].target.window_id.as_deref(), Some("win-1"));
        assert_eq!(listed[0].target.session_id.as_deref(), Some("sess-1"));
        assert_eq!(listed[0].target.project_root.as_deref(), Some("/tmp/repo"));
        assert_eq!(listed[0].schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn list_since_returns_only_rows_at_or_after_the_cutoff() {
        let (_dir, _home) = isolated_home();
        let mut older = sample(ErrorKind::HookFailure, "older hook");
        older.recorded_at = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let mut newer = sample(ErrorKind::DaemonFault, "newer daemon");
        newer.recorded_at = Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0).unwrap();
        record(older).expect("older");
        record(newer.clone()).expect("newer");

        let listed = list_since(Some(newer.recorded_at)).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, newer.id);
        assert_eq!(listed[0].kind, ErrorKind::DaemonFault);
    }

    #[test]
    fn ledger_is_append_only_across_kinds() {
        let (_dir, _home) = isolated_home();
        record(sample(ErrorKind::OperationRefusal, "first")).expect("first");
        record(sample(ErrorKind::LaunchFailure, "second")).expect("second");

        let listed = list_since(None).expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].message, "first");
        assert_eq!(listed[1].message, "second");
    }

    #[test]
    fn kind_serializes_as_snake_case() {
        let json = serde_json::to_string(&ErrorKind::OperationRefusal).expect("json");
        assert_eq!(json, "\"operation_refusal\"");
        let json = serde_json::to_string(&ErrorKind::LaunchFailure).expect("json");
        assert_eq!(json, "\"launch_failure\"");
        let json = serde_json::to_string(&ErrorKind::HookFailure).expect("json");
        assert_eq!(json, "\"hook_failure\"");
        let json = serde_json::to_string(&ErrorKind::DaemonFault).expect("json");
        assert_eq!(json, "\"daemon_fault\"");
    }

    #[test]
    fn context_values_roundtrip_and_are_sanitized() {
        let (_dir, _home) = isolated_home();
        let mut context = std::collections::BTreeMap::new();
        context.insert("event".to_string(), "PreToolUse".to_string());
        context.insert(
            "detail".to_string(),
            "Bearer ghp_secret0123456789abcdef \u{1b}[31mred\u{1b}[0m".to_string(),
        );
        let recorded = record(
            ErrorRecord::new(ErrorKind::HookFailure, "hook", ErrorTarget::default())
                .with_context(context),
        )
        .expect("record");

        let listed = list_since(None).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, recorded.id);
        assert_eq!(
            listed[0].context.get("event").map(String::as_str),
            Some("PreToolUse")
        );
        let detail = listed[0].context.get("detail").expect("detail");
        assert!(!detail.contains("ghp_secret"), "{detail}");
        assert!(!detail.contains('\u{1b}'), "{detail}");
        assert!(
            !detail.contains("[31m"),
            "ANSI remnants must be stripped: {detail}"
        );
        assert!(detail.contains("red"), "{detail}");
    }

    #[test]
    fn rows_without_context_still_deserialize() {
        let raw = r#"{"schema_version":1,"id":"row-1","recorded_at":"2026-08-30T00:00:00Z","kind":"hook_failure","message":"legacy","target":{}}"#;
        let record: ErrorRecord = serde_json::from_str(raw).expect("legacy row");
        assert!(record.context.is_empty());
    }

    #[test]
    fn messages_redact_secrets_and_drop_control_chars() {
        let record = ErrorRecord::new(
            ErrorKind::OperationRefusal,
            "token GITHUB_TOKEN=ghp_secret\u{0007} boom",
            ErrorTarget::default(),
        );
        assert!(!record.message.contains("ghp_secret"));
        assert!(record.message.contains("[REDACTED]"));
        assert!(!record.message.contains('\u{0007}'));
    }
}
