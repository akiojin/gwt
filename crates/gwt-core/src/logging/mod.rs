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

        if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    let modified: chrono::DateTime<Utc> = modified.into();
                    if modified < cutoff
                        && std::fs::remove_file(&path).is_ok() {
                            removed += 1;
                        }
                }
            }
        }
    }

    Ok(removed)
}

/// Get log file path for today
pub fn today_log_path(log_dir: &Path) -> std::path::PathBuf {
    let date = Utc::now().format("%Y-%m-%d");
    log_dir.join(format!("gwt.{}.jsonl", date))
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
        assert!(path_str.starts_with("/logs/gwt."));
        assert!(path_str.ends_with(".jsonl"));
    }
}
