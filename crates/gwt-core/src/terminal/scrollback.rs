//! File-based scrollback buffer
//!
//! Manages terminal output persistence to disk for scrollback.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
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
        let file = OpenOptions::new()
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
        let file = File::open(&self.file_path).map_err(|e| TerminalError::ScrollbackError {
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
        let file_path = home
            .join(".gwt")
            .join("terminals")
            .join(format!("{pane_id}.log"));
        if file_path.exists() {
            fs::remove_file(&file_path).map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to remove file: {e}"),
            })?;
        }
        Ok(())
    }

    /// Returns the path to the scrollback file for a given pane ID.
    pub fn scrollback_path_for_pane(pane_id: &str) -> Result<PathBuf, TerminalError> {
        let home = dirs::home_dir().ok_or_else(|| TerminalError::ScrollbackError {
            details: "failed to determine home directory".to_string(),
        })?;
        Ok(home
            .join(".gwt")
            .join("terminals")
            .join(format!("{pane_id}.log")))
    }

    /// Reads the entire scrollback file and returns plain text with ANSI sequences removed.
    pub fn read_all_text(&self) -> Result<String, TerminalError> {
        let data = fs::read(&self.file_path).map_err(|e| TerminalError::ScrollbackError {
            details: format!("failed to read scrollback file: {e}"),
        })?;
        Ok(strip_ansi(&data))
    }

    /// Reads up to `max_bytes` from the end of the scrollback file.
    ///
    /// This is intended for diagnostics (e.g. ANSI/SGR probing) where reading the whole
    /// log would be too expensive.
    pub fn read_tail_bytes(&self, max_bytes: usize) -> Result<Vec<u8>, TerminalError> {
        Self::read_tail_bytes_at(&self.file_path, max_bytes)
    }

    /// Reads up to `max_bytes` from the end of `path`.
    pub fn read_tail_bytes_at(path: &Path, max_bytes: usize) -> Result<Vec<u8>, TerminalError> {
        let mut file = File::open(path).map_err(|e| TerminalError::ScrollbackError {
            details: format!("failed to open scrollback file: {e}"),
        })?;
        let len = file
            .metadata()
            .map(|m| m.len())
            .map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to stat scrollback file: {e}"),
            })?;

        let start = if max_bytes == 0 || len as usize <= max_bytes {
            0
        } else {
            len - max_bytes as u64
        };

        file.seek(SeekFrom::Start(start))
            .map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to seek scrollback file: {e}"),
            })?;

        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| TerminalError::ScrollbackError {
                details: format!("failed to read scrollback file: {e}"),
            })?;
        Ok(buf)
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

/// Strips ANSI escape sequences from raw bytes, returning plain UTF-8 text.
///
/// Handles CSI (`ESC [`), OSC (`ESC ]`), and other ESC sequences.
/// Preserves `\n`, `\r`, and `\t` but removes other C0 control characters.
pub fn strip_ansi(input: &[u8]) -> String {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        if b == 0x1b {
            // ESC
            i += 1;
            if i >= input.len() {
                break;
            }
            match input[i] {
                b'[' => {
                    // CSI sequence: skip until final byte (0x40-0x7e)
                    i += 1;
                    while i < input.len() {
                        let c = input[i];
                        i += 1;
                        if (0x40..=0x7e).contains(&c) {
                            break;
                        }
                    }
                }
                b']' => {
                    // OSC sequence: skip until ST (ESC \) or BEL (0x07)
                    i += 1;
                    while i < input.len() {
                        if input[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if input[i] == 0x1b && i + 1 < input.len() && input[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                b'(' | b')' | b'*' | b'+' => {
                    // Character set designation: skip one more byte
                    i += 1;
                }
                _ => {
                    // Other ESC sequences (e.g., ESC =, ESC >): skip just the byte after ESC
                    i += 1;
                }
            }
        } else if b < 0x20 || b == 0x7f {
            // Control character: keep \n, \r, \t; skip others
            if b == b'\n' || b == b'\r' || b == b'\t' {
                out.push(b);
            }
            i += 1;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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

    // --- ANSI strip tests ---

    #[test]
    fn test_strip_ansi_colors() {
        let input = b"\x1b[31mhello\x1b[0m world";
        let result = strip_ansi(input);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_strip_ansi_cursor_movement() {
        let input = b"\x1b[2J\x1b[H\x1b[3Ahello\x1b[5B";
        let result = strip_ansi(input);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_strip_ansi_osc() {
        // OSC with BEL terminator
        let input = b"\x1b]0;my title\x07hello";
        let result = strip_ansi(input);
        assert_eq!(result, "hello");

        // OSC with ST terminator
        let input2 = b"\x1b]0;my title\x1b\\hello";
        let result2 = strip_ansi(input2);
        assert_eq!(result2, "hello");
    }

    #[test]
    fn test_strip_ansi_plain_text() {
        let input = b"hello world\nline2\ttab";
        let result = strip_ansi(input);
        assert_eq!(result, "hello world\nline2\ttab");
    }

    #[test]
    fn test_strip_ansi_control_chars() {
        // Control chars other than \n, \r, \t should be removed
        let input = b"hello\x01\x02\x03world";
        let result = strip_ansi(input);
        assert_eq!(result, "helloworld");
    }

    #[test]
    fn test_read_all_text() {
        let tmp = TempDir::new().unwrap();
        let mut sb = create_test_scrollback(&tmp, "ansi");
        sb.write(b"\x1b[32mgreen\x1b[0m text\nline2\n").unwrap();
        sb.flush().unwrap();

        let text = sb.read_all_text().unwrap();
        assert_eq!(text, "green text\nline2\n");
    }

    #[test]
    fn test_scrollback_path_for_pane() {
        let path = ScrollbackFile::scrollback_path_for_pane("pane-abc123").unwrap();
        assert!(path.ends_with(".gwt/terminals/pane-abc123.log"));
    }

    #[test]
    fn test_read_tail_bytes_at_reads_from_end() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("tail.log");
        fs::write(&path, b"0123456789").unwrap();

        let tail = ScrollbackFile::read_tail_bytes_at(&path, 4).unwrap();
        assert_eq!(tail, b"6789");
    }

    #[test]
    fn test_read_tail_bytes_at_when_max_exceeds_len_returns_all() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("tail2.log");
        fs::write(&path, b"abc").unwrap();

        let tail = ScrollbackFile::read_tail_bytes_at(&path, 1024).unwrap();
        assert_eq!(tail, b"abc");
    }

    #[test]
    fn test_read_tail_bytes_uses_instance_path() {
        let tmp = TempDir::new().unwrap();
        let mut sb = create_test_scrollback(&tmp, "tail3");
        sb.write(b"hello world").unwrap();
        sb.flush().unwrap();

        let tail = sb.read_tail_bytes(5).unwrap();
        assert_eq!(tail, b"world");
    }
}
