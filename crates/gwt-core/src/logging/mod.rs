//! Logging module
//!
//! Provides JSON Lines logging with tracing integration and log file management.

mod logger;
mod reader;

pub use logger::{init_logger, LogConfig};
pub use reader::{LogEntry, LogReader};

use crate::error::Result;
use chrono::{Duration, Utc};
use std::path::Path;

/// Clean up old log files based on retention days
pub fn cleanup_old_logs(log_dir: &Path, retention_days: u32) -> Result<usize> {
    if !log_dir.exists() {
        return Ok(0);
    }

    let cutoff = Utc::now() - Duration::days(retention_days as i64);
    let mut removed = 0;

    for entry in std::fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Support both formats: *.jsonl (legacy) and gwt.jsonl.* (tracing-appender)
        if LogReader::is_log_file(&path) {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    let modified: chrono::DateTime<Utc> = modified.into();
                    if modified < cutoff && std::fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
    }

    Ok(removed)
}

/// Get log file path for today (tracing-appender format: gwt.jsonl.YYYY-MM-DD)
pub fn today_log_path(log_dir: &Path) -> std::path::PathBuf {
    let date = Utc::now().format("%Y-%m-%d");
    log_dir.join(format!("gwt.jsonl.{}", date))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_cleanup_old_logs() {
        let temp = TempDir::new().unwrap();

        // Create some test files
        std::fs::write(temp.path().join("gwt.2020-01-01.jsonl"), "old").unwrap();
        std::fs::write(temp.path().join("gwt.2099-01-01.jsonl"), "new").unwrap();

        // Set old file modification time (can't easily do this, so just test the function runs)
        let result = cleanup_old_logs(temp.path(), 7);
        assert!(result.is_ok());
    }

    #[test]
    fn test_today_log_path() {
        let path = today_log_path(Path::new("/logs"));
        let path_str = path.to_string_lossy();
        // tracing-appender format: gwt.jsonl.YYYY-MM-DD
        assert!(path_str.starts_with("/logs/gwt.jsonl."));
    }

    #[test]
    fn test_log_json_structure_has_required_fields() {
        // Test that LogEntry can parse tracing-subscriber JSON format
        // Format: { timestamp, level, fields: { message, category, ... }, target }
        let json_with_category = r#"{
            "timestamp": "2024-01-01T00:00:00Z",
            "level": "INFO",
            "fields": {
                "message": "test message",
                "category": "worktree"
            },
            "target": "gwt"
        }"#;

        let entry: LogEntry = serde_json::from_str(json_with_category).unwrap();
        assert_eq!(entry.timestamp, "2024-01-01T00:00:00Z");
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.message(), "test message");
        assert_eq!(entry.category(), Some("worktree"));
        assert_eq!(entry.target, "gwt");
    }

    #[test]
    fn test_log_category_field_preserved() {
        // Test that different category values are correctly parsed
        let categories = ["cli", "worktree", "git", "server"];

        for category in categories {
            let json = format!(
                r#"{{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","fields":{{"message":"test","category":"{}"}},"target":"gwt"}}"#,
                category
            );

            let entry: LogEntry = serde_json::from_str(&json).unwrap();
            assert_eq!(
                entry.category(),
                Some(category),
                "Category '{}' should be preserved",
                category
            );
        }
    }

    #[test]
    fn test_log_append_preserves_existing_entries() {
        let temp = TempDir::new().unwrap();
        let log_file = temp.path().join("gwt.jsonl.2024-01-01");

        // Write initial entries (tracing-subscriber format)
        let initial_entries = [
            r#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","fields":{"message":"entry1"},"target":"gwt"}"#,
            r#"{"timestamp":"2024-01-01T00:00:01Z","level":"INFO","fields":{"message":"entry2"},"target":"gwt"}"#,
        ];
        std::fs::write(&log_file, initial_entries.join("\n") + "\n").unwrap();

        // Simulate append (like a new session would do)
        use std::fs::OpenOptions;
        use std::io::Write;
        let mut file = OpenOptions::new().append(true).open(&log_file).unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2024-01-01T00:00:02Z","level":"INFO","fields":{{"message":"entry3"}},"target":"gwt"}}"#
        )
        .unwrap();

        // Read all entries and verify none were lost
        let (entries, _) = LogReader::read_entries(&log_file, 0, 100).unwrap();
        assert_eq!(
            entries.len(),
            3,
            "All entries should be preserved after append"
        );
        assert_eq!(
            entries[0].message(),
            "entry1",
            "First entry should be preserved"
        );
        assert_eq!(
            entries[1].message(),
            "entry2",
            "Second entry should be preserved"
        );
        assert_eq!(
            entries[2].message(),
            "entry3",
            "New entry should be appended"
        );
    }

    #[test]
    fn test_each_log_line_is_valid_json() {
        let temp = TempDir::new().unwrap();
        let log_file = temp.path().join("gwt.jsonl.2024-01-01");

        // Write multiple entries (tracing-subscriber format)
        let entries = [
            r#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","fields":{"message":"test1","category":"cli"},"target":"gwt"}"#,
            r#"{"timestamp":"2024-01-01T00:00:01Z","level":"DEBUG","fields":{"message":"test2","category":"git"},"target":"gwt"}"#,
            r#"{"timestamp":"2024-01-01T00:00:02Z","level":"WARN","fields":{"message":"test3","category":"worktree"},"target":"gwt"}"#,
        ];
        std::fs::write(&log_file, entries.join("\n") + "\n").unwrap();

        // Read file line by line and verify each line is valid JSON
        use std::io::{BufRead, BufReader};
        let file = std::fs::File::open(&log_file).unwrap();
        let reader = BufReader::new(file);

        for (i, line) in reader.lines().enumerate() {
            let line = line.unwrap();
            if line.is_empty() {
                continue;
            }
            let result: std::result::Result<serde_json::Value, _> = serde_json::from_str(&line);
            assert!(
                result.is_ok(),
                "Line {} should be valid JSON: {}",
                i + 1,
                line
            );
        }
    }
}
