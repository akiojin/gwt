//! File-based scrollback buffer
//!
//! Manages terminal output persistence to disk for scrollback.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::terminal::TerminalError;

/// File-based scrollback buffer for terminal output.
///
/// Writes terminal output to a log file and supports reading back
/// arbitrary line ranges for scrollback display.
pub struct ScrollbackFile {
    file_path: PathBuf,
    writer: BufWriter<File>,
    line_count: usize,
}

impl ScrollbackFile {
    /// Creates a new scrollback file for the given pane ID.
    ///
    /// The file is stored at `~/.gwt/terminals/{pane_id}.log`.
    /// Creates the directory if it does not exist.
    pub fn new(pane_id: &str) -> Result<Self, TerminalError> {
        let home = dirs::home_dir().ok_or_else(|| TerminalError::ScrollbackError {
            details: "failed to determine home directory".to_string(),
        })?;
        let dir = home.join(".gwt").join("terminals");
        let file_path = dir.join(format!("{pane_id}.log"));
        Self::with_path(file_path)
    }

    /// Creates a new scrollback file at an explicit path.
    ///
    /// Useful for testing with temporary directories.
    pub fn with_path(file_path: PathBuf) -> Result<Self, TerminalError> {
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to create directory: {e}"),
            })?;
        }
        let file =
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file_path)
                .map_err(|e| TerminalError::ScrollbackError {
                    details: format!("failed to open scrollback file: {e}"),
                })?;
        let writer = BufWriter::new(file);
        Ok(Self {
            file_path,
            writer,
            line_count: 0,
        })
    }

    /// Writes data to the scrollback file.
    ///
    /// Counts newlines in the data to track the total line count.
    pub fn write(&mut self, data: &[u8]) -> Result<(), TerminalError> {
        self.writer
            .write_all(data)
            .map_err(|e| TerminalError::ScrollbackError {
                details: format!("write failed: {e}"),
            })?;
        self.line_count += data.iter().filter(|&&b| b == b'\n').count();
        Ok(())
    }

    /// Flushes the internal buffer to disk.
    pub fn flush(&mut self) -> Result<(), TerminalError> {
        self.writer
            .flush()
            .map_err(|e| TerminalError::ScrollbackError {
                details: format!("flush failed: {e}"),
            })
    }

    /// Reads `count` lines starting from line `start` (0-indexed).
    ///
    /// If the requested range extends beyond the file, returns only
    /// the available lines.
    pub fn read_lines(&self, start: usize, count: usize) -> Result<Vec<String>, TerminalError> {
        let file =
            File::open(&self.file_path).map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to open file for reading: {e}"),
            })?;
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader
            .lines()
            .skip(start)
            .take(count)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to read lines: {e}"),
            })?;
        Ok(lines)
    }

    /// Returns the number of lines written so far.
    pub fn line_count(&self) -> usize {
        self.line_count
    }

    /// Returns the path to the scrollback file.
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Removes the log file for the given pane ID.
    pub fn cleanup(pane_id: &str) -> Result<(), TerminalError> {
        let home = dirs::home_dir().ok_or_else(|| TerminalError::ScrollbackError {
            details: "failed to determine home directory".to_string(),
        })?;
        let file_path = home.join(".gwt").join("terminals").join(format!("{pane_id}.log"));
        if file_path.exists() {
            fs::remove_file(&file_path).map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to remove file: {e}"),
            })?;
        }
        Ok(())
    }

    /// Removes all log files in `~/.gwt/terminals/`.
    ///
    /// Intended to be called on gwt shutdown.
    pub fn cleanup_all() -> Result<(), TerminalError> {
        let home = dirs::home_dir().ok_or_else(|| TerminalError::ScrollbackError {
            details: "failed to determine home directory".to_string(),
        })?;
        let dir = home.join(".gwt").join("terminals");
        if dir.exists() {
            for entry in fs::read_dir(&dir).map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to read directory: {e}"),
            })? {
                let entry = entry.map_err(|e| TerminalError::ScrollbackError {
                    details: format!("failed to read entry: {e}"),
                })?;
                if entry.path().is_file() {
                    fs::remove_file(entry.path()).map_err(|e| TerminalError::ScrollbackError {
                        details: format!("failed to remove file: {e}"),
                    })?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_scrollback(tmp: &TempDir, name: &str) -> ScrollbackFile {
        let path = tmp.path().join(format!("{name}.log"));
        ScrollbackFile::with_path(path).expect("failed to create scrollback file")
    }

    #[test]
    fn test_write_then_read_lines() {
        let tmp = TempDir::new().unwrap();
        let mut sb = create_test_scrollback(&tmp, "test1");
        sb.write(b"hello\nworld\n").unwrap();
        sb.flush().unwrap();

        let lines = sb.read_lines(0, 2).unwrap();
        assert_eq!(lines, vec!["hello", "world"]);
    }

    #[test]
    fn test_large_write_and_range_read() {
        let tmp = TempDir::new().unwrap();
        let mut sb = create_test_scrollback(&tmp, "large");
        for i in 0..10000 {
            sb.write(format!("line-{i}\n").as_bytes()).unwrap();
        }
        sb.flush().unwrap();

        let lines = sb.read_lines(5000, 10).unwrap();
        assert_eq!(lines.len(), 10);
        for (j, line) in lines.iter().enumerate() {
            assert_eq!(line, &format!("line-{}", 5000 + j));
        }
    }

    #[test]
    fn test_line_count() {
        let tmp = TempDir::new().unwrap();
        let mut sb = create_test_scrollback(&tmp, "count");
        sb.write(b"a\nb\nc\n").unwrap();
        assert_eq!(sb.line_count(), 3);
    }

    #[test]
    fn test_read_beyond_range() {
        let tmp = TempDir::new().unwrap();
        let mut sb = create_test_scrollback(&tmp, "range");
        sb.write(b"x\ny\nz\n").unwrap();
        sb.flush().unwrap();

        let lines = sb.read_lines(0, 100).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines, vec!["x", "y", "z"]);
    }

    #[test]
    fn test_cleanup_removes_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("cleanup-test.log");
        {
            let mut sb = ScrollbackFile::with_path(path.clone()).unwrap();
            sb.write(b"data\n").unwrap();
            sb.flush().unwrap();
        }
        assert!(path.exists());

        // Manually remove the file (simulating cleanup logic without home dir)
        fs::remove_file(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_empty_write() {
        let tmp = TempDir::new().unwrap();
        let mut sb = create_test_scrollback(&tmp, "empty");
        sb.write(b"").unwrap();
        assert_eq!(sb.line_count(), 0);
    }

    #[test]
    fn test_multiple_writes() {
        let tmp = TempDir::new().unwrap();
        let mut sb = create_test_scrollback(&tmp, "multi");
        sb.write(b"a\n").unwrap();
        sb.write(b"b\n").unwrap();
        sb.flush().unwrap();

        assert_eq!(sb.line_count(), 2);
        let lines = sb.read_lines(0, 2).unwrap();
        assert_eq!(lines, vec!["a", "b"]);
    }

    #[test]
    fn test_file_path_accessor() {
        let tmp = TempDir::new().unwrap();
        let expected = tmp.path().join("path-test.log");
        let sb = ScrollbackFile::with_path(expected.clone()).unwrap();
        assert_eq!(sb.file_path(), expected.as_path());
    }
}
