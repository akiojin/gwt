//! Log reader for viewing historical logs

use crate::error::Result;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Log entry from JSON Lines file
#[derive(Debug, Clone, Deserialize)]
pub struct LogEntry {
    /// Timestamp
    pub timestamp: String,
    /// Log level
    pub level: String,
    /// Log message
    pub message: String,
    /// Target module
    #[serde(default)]
    pub target: String,
    /// Span information
    #[serde(default)]
    pub span: Option<serde_json::Value>,
    /// Additional fields
    #[serde(flatten)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

/// Log reader for lazy loading of log files
pub struct LogReader {
    /// Log directory
    log_dir: PathBuf,
}

impl LogReader {
    /// Create a new log reader
    pub fn new(log_dir: impl Into<PathBuf>) -> Self {
        Self {
            log_dir: log_dir.into(),
        }
    }

    /// List available log files
    pub fn list_files(&self) -> Result<Vec<PathBuf>> {
        if !self.log_dir.exists() {
            return Ok(Vec::new());
        }

        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.log_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "jsonl").unwrap_or(false))
            .collect();

        files.sort_by(|a, b| b.cmp(a)); // Newest first
        Ok(files)
    }

    /// Read entries from a log file with pagination
    pub fn read_entries(
        path: &Path,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<LogEntry>, bool)> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut entries = Vec::new();
        let mut has_more = false;

        for (i, line) in reader.lines().enumerate() {
            if i < offset {
                continue;
            }
            if entries.len() >= limit {
                has_more = true;
                break;
            }

            let line = line?;
            if let Ok(entry) = serde_json::from_str::<LogEntry>(&line) {
                entries.push(entry);
            }
        }

        Ok((entries, has_more))
    }

    /// Read the latest N entries across all log files
    pub fn read_latest(&self, limit: usize) -> Result<Vec<LogEntry>> {
        let files = self.list_files()?;
        let mut entries = Vec::new();

        for file in files {
            if entries.len() >= limit {
                break;
            }

            let remaining = limit - entries.len();
            let (file_entries, _) = Self::read_entries(&file, 0, remaining)?;
            entries.extend(file_entries);
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_list_files() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("gwt.2024-01-01.jsonl"), "").unwrap();
        std::fs::write(temp.path().join("gwt.2024-01-02.jsonl"), "").unwrap();

        let reader = LogReader::new(temp.path());
        let files = reader.list_files().unwrap();

        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_read_entries() {
        let temp = TempDir::new().unwrap();
        let log_file = temp.path().join("test.jsonl");

        let entries = [r#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","message":"test1"}"#,
            r#"{"timestamp":"2024-01-01T00:00:01Z","level":"DEBUG","message":"test2"}"#];
        std::fs::write(&log_file, entries.join("\n")).unwrap();

        let (read_entries, has_more) = LogReader::read_entries(&log_file, 0, 10).unwrap();
        assert_eq!(read_entries.len(), 2);
        assert!(!has_more);

        let (read_entries, has_more) = LogReader::read_entries(&log_file, 0, 1).unwrap();
        assert_eq!(read_entries.len(), 1);
        assert!(has_more);
    }
}
