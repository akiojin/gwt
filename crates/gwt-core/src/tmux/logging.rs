//! tmux pane logging
//!
//! Provides functionality to capture and store logs from tmux panes.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::error::{TmuxError, TmuxResult};

/// Log capture configuration
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Directory to store log files
    pub log_dir: PathBuf,
    /// Maximum log file size in bytes (0 = unlimited)
    pub max_size: u64,
    /// Whether to enable logging
    pub enabled: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            log_dir: PathBuf::from("/tmp/gwt-logs"),
            max_size: 10 * 1024 * 1024, // 10MB
            enabled: true,
        }
    }
}

impl LogConfig {
    /// Create a new log configuration
    pub fn new(log_dir: PathBuf) -> Self {
        Self {
            log_dir,
            ..Default::default()
        }
    }

    /// Generate log file path for a pane
    pub fn log_path(&self, session: &str, pane_id: &str) -> PathBuf {
        let sanitized_pane = pane_id.replace('%', "");
        self.log_dir
            .join(format!("{}-{}.log", session, sanitized_pane))
    }

    /// Ensure log directory exists
    pub fn ensure_dir(&self) -> io::Result<()> {
        if !self.log_dir.exists() {
            fs::create_dir_all(&self.log_dir)?;
        }
        Ok(())
    }
}

/// Start logging for a pane using tmux pipe-pane
///
/// This captures the pane's output to a log file.
pub fn start_logging(pane_id: &str, log_path: &Path) -> TmuxResult<()> {
    // Ensure parent directory exists
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|e| TmuxError::CommandFailed {
            command: "create log dir".to_string(),
            reason: e.to_string(),
        })?;
    }

    let output = Command::new("tmux")
        .args([
            "pipe-pane",
            "-t",
            pane_id,
            &format!("cat >> {}", log_path.display()),
        ])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "pipe-pane".to_string(),
            reason: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TmuxError::CommandFailed {
            command: "pipe-pane".to_string(),
            reason: stderr.to_string(),
        });
    }

    Ok(())
}

/// Stop logging for a pane
pub fn stop_logging(pane_id: &str) -> TmuxResult<()> {
    let output = Command::new("tmux")
        .args(["pipe-pane", "-t", pane_id])
        .output()
        .map_err(|e| TmuxError::CommandFailed {
            command: "pipe-pane".to_string(),
            reason: e.to_string(),
        })?;

    // Not an error if pipe wasn't active
    if !output.status.success() {
        return Ok(());
    }

    Ok(())
}

/// Read the contents of a log file
pub fn read_log(log_path: &Path) -> io::Result<String> {
    fs::read_to_string(log_path)
}

/// Read the last N lines of a log file
pub fn read_log_tail(log_path: &Path, lines: usize) -> io::Result<Vec<String>> {
    // Logs are capped by max_size (default 10MB), so full read is acceptable here.
    let content = fs::read_to_string(log_path)?;
    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(lines);
    Ok(all_lines[start..].iter().map(|s| s.to_string()).collect())
}

/// Append a message to a log file
pub fn append_log(log_path: &Path, message: &str) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    writeln!(file, "{}", message)?;
    Ok(())
}

/// Get the size of a log file
pub fn log_size(log_path: &Path) -> io::Result<u64> {
    Ok(fs::metadata(log_path)?.len())
}

/// Truncate a log file if it exceeds the max size
pub fn truncate_if_needed(log_path: &Path, max_size: u64) -> io::Result<bool> {
    if max_size == 0 {
        return Ok(false);
    }

    let current_size = log_size(log_path)?;
    if current_size > max_size {
        // Read last portion of the file
        let content = fs::read_to_string(log_path)?;
        let lines: Vec<&str> = content.lines().collect();
        let keep_lines = lines.len() / 2; // Keep last half
        let truncated: String = lines[lines.len() - keep_lines..].join("\n");

        // Rewrite the file
        let mut file = File::create(log_path)?;
        writeln!(file, "--- Log truncated ---")?;
        write!(file, "{}", truncated)?;

        return Ok(true);
    }

    Ok(false)
}

/// Clean up old log files
pub fn cleanup_old_logs(log_dir: &Path, max_age_secs: u64) -> io::Result<usize> {
    let mut removed = 0;

    if !log_dir.exists() {
        return Ok(0);
    }

    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "log") {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = modified.elapsed() {
                        if age.as_secs() > max_age_secs {
                            fs::remove_file(&path)?;
                            removed += 1;
                        }
                    }
                }
            }
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_log_dir() -> PathBuf {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        env::temp_dir().join(format!("gwt-test-logs-{}-{}", std::process::id(), count))
    }

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_size, 10 * 1024 * 1024);
    }

    #[test]
    fn test_log_config_log_path() {
        let config = LogConfig::new(PathBuf::from("/tmp/logs"));
        let path = config.log_path("gwt-test", "%5");
        assert_eq!(path, PathBuf::from("/tmp/logs/gwt-test-5.log"));
    }

    #[test]
    fn test_log_config_ensure_dir() {
        let log_dir = temp_log_dir();
        let config = LogConfig::new(log_dir.clone());

        // Clean up if exists
        let _ = fs::remove_dir_all(&log_dir);

        assert!(config.ensure_dir().is_ok());
        assert!(log_dir.exists());

        // Clean up
        let _ = fs::remove_dir_all(&log_dir);
    }

    #[test]
    fn test_append_and_read_log() {
        let log_dir = temp_log_dir();
        let _ = fs::create_dir_all(&log_dir);
        let log_path = log_dir.join("test.log");

        // Clean up if exists
        let _ = fs::remove_file(&log_path);

        // Append
        assert!(append_log(&log_path, "line 1").is_ok());
        assert!(append_log(&log_path, "line 2").is_ok());
        assert!(append_log(&log_path, "line 3").is_ok());

        // Read
        let content = read_log(&log_path).unwrap();
        assert!(content.contains("line 1"));
        assert!(content.contains("line 2"));
        assert!(content.contains("line 3"));

        // Clean up
        let _ = fs::remove_file(&log_path);
        let _ = fs::remove_dir_all(&log_dir);
    }

    #[test]
    fn test_read_log_tail() {
        let log_dir = temp_log_dir();
        let _ = fs::create_dir_all(&log_dir);
        let log_path = log_dir.join("tail-test.log");

        // Create log with multiple lines
        let mut file = File::create(&log_path).unwrap();
        for i in 1..=10 {
            writeln!(file, "line {}", i).unwrap();
        }
        drop(file);

        // Read last 3 lines
        let lines = read_log_tail(&log_path, 3).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "line 8");
        assert_eq!(lines[1], "line 9");
        assert_eq!(lines[2], "line 10");

        // Clean up
        let _ = fs::remove_file(&log_path);
        let _ = fs::remove_dir_all(&log_dir);
    }

    #[test]
    fn test_log_size() {
        let log_dir = temp_log_dir();
        let _ = fs::create_dir_all(&log_dir);
        let log_path = log_dir.join("size-test.log");

        // Create log
        let mut file = File::create(&log_path).unwrap();
        write!(file, "12345").unwrap();
        drop(file);

        let size = log_size(&log_path).unwrap();
        assert_eq!(size, 5);

        // Clean up
        let _ = fs::remove_file(&log_path);
        let _ = fs::remove_dir_all(&log_dir);
    }

    #[test]
    fn test_truncate_if_needed_under_limit() {
        let log_dir = temp_log_dir();
        let _ = fs::create_dir_all(&log_dir);
        let log_path = log_dir.join("truncate-test.log");

        // Create small log
        let mut file = File::create(&log_path).unwrap();
        writeln!(file, "small content").unwrap();
        drop(file);

        // Should not truncate
        let truncated = truncate_if_needed(&log_path, 1000).unwrap();
        assert!(!truncated);

        // Clean up
        let _ = fs::remove_file(&log_path);
        let _ = fs::remove_dir_all(&log_dir);
    }

    #[test]
    fn test_cleanup_old_logs() {
        let log_dir = temp_log_dir();
        let _ = fs::create_dir_all(&log_dir);

        // Create a recent log
        let recent_path = log_dir.join("recent.log");
        File::create(&recent_path).unwrap();

        // Cleanup with very long max age should remove nothing
        let removed = cleanup_old_logs(&log_dir, 999999).unwrap();
        assert_eq!(removed, 0);

        // Clean up
        let _ = fs::remove_file(&recent_path);
        let _ = fs::remove_dir_all(&log_dir);
    }
}
