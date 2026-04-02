//! Terminal pane: integrates PTY handle + vt100 parser + scrollback.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::pty::{PtyHandle, SpawnConfig};
use crate::scrollback::{ScrollbackLine, ScrollbackStorage};
use crate::TerminalError;

/// Status of a pane's child process.
#[derive(Debug, Clone, PartialEq)]
pub enum PaneStatus {
    Running,
    Completed(i32),
    Error(String),
}

/// A terminal pane integrating PTY, vt100 parser, and scrollback.
pub struct Pane {
    id: String,
    pty: PtyHandle,
    parser: vt100::Parser,
    scrollback: ScrollbackStorage,
    status: PaneStatus,
    /// Accumulator for incomplete lines from raw PTY output.
    line_buf: String,
}

impl Pane {
    /// Create a new pane by spawning a PTY process.
    pub fn new(
        id: String,
        command: String,
        args: Vec<String>,
        cols: u16,
        rows: u16,
        env: HashMap<String, String>,
        cwd: Option<PathBuf>,
    ) -> Result<Self, TerminalError> {
        let config = SpawnConfig {
            command,
            args,
            cols,
            rows,
            env,
            cwd,
        };
        let pty = PtyHandle::spawn(config)?;
        let parser = vt100::Parser::new(rows, cols, 0);
        let scrollback = ScrollbackStorage::new(ScrollbackStorage::DEFAULT_CAPACITY);

        Ok(Self {
            id,
            pty,
            parser,
            scrollback,
            status: PaneStatus::Running,
            line_buf: String::new(),
        })
    }

    /// Get the pane ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Alias for `id()` — used by gwt-tui.
    pub fn pane_id(&self) -> &str {
        &self.id
    }

    /// Get a reference to the PTY handle.
    pub fn pty(&self) -> &PtyHandle {
        &self.pty
    }

    /// Feed raw bytes from PTY output through the vt100 parser and scrollback.
    ///
    /// The vt100 parser is the single source of truth for terminal screen state.
    /// Completed lines (delimited by `\n`) are also captured into the scrollback
    /// ring buffer for history access.
    pub fn process_bytes(&mut self, data: &[u8]) {
        // Update vt100 screen state
        self.parser.process(data);

        // Capture lines for scrollback
        let text = String::from_utf8_lossy(data);
        self.line_buf.push_str(&text);

        // Split on newlines and push completed lines into the ring buffer.
        // Uses drain to avoid repeated allocation from slicing.
        while let Some(pos) = self.line_buf.find('\n') {
            let line: String = self.line_buf.drain(..pos).collect();
            self.line_buf.drain(..1); // consume the '\n'
            self.scrollback.push_line(ScrollbackLine {
                text: line,
                wrapped: false,
            });
        }
    }

    /// Get the current vt100 screen.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Get scrollback lines from the ring buffer.
    pub fn scrollback_lines(&self, start: usize, count: usize) -> Vec<&ScrollbackLine> {
        self.scrollback.get_lines(start, count)
    }

    /// Total number of lines in scrollback.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Get the current pane status.
    pub fn status(&self) -> &PaneStatus {
        &self.status
    }

    /// Check and update the pane's process status.
    pub fn check_status(&mut self) -> Result<&PaneStatus, TerminalError> {
        if self.status == PaneStatus::Running {
            if let Some(exit_status) = self.pty.try_wait()? {
                if exit_status.success() {
                    self.status = PaneStatus::Completed(0);
                } else {
                    self.status = PaneStatus::Completed(1);
                }
            }
        }
        Ok(&self.status)
    }

    /// Mark this pane as errored.
    pub fn mark_error(&mut self, message: impl Into<String>) {
        self.status = PaneStatus::Error(message.into());
    }

    /// Write input to the PTY.
    pub fn write_input(&self, data: &[u8]) -> Result<(), TerminalError> {
        self.pty.write_input(data)
    }

    /// Resize the pane (PTY + vt100 parser).
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError> {
        self.pty.resize(cols, rows)?;
        self.parser.set_size(rows, cols);
        Ok(())
    }

    /// Kill the child process.
    pub fn kill(&self) -> Result<(), TerminalError> {
        self.pty.kill()
    }

    /// Get a reader for the PTY output.
    pub fn reader(&self) -> Result<Box<dyn std::io::Read + Send>, TerminalError> {
        self.pty.reader()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::read_with_timeout;
    use std::time::Duration;

    #[test]
    fn test_pane_creation() {
        let pane = Pane::new(
            "test-1".to_string(),
            "/bin/echo".to_string(),
            vec!["hello".to_string()],
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("Pane creation failed");

        assert_eq!(pane.id(), "test-1");
        assert_eq!(pane.status(), &PaneStatus::Running);
        assert_eq!(pane.scrollback_len(), 0);
    }

    #[test]
    fn test_process_bytes_updates_screen() {
        let mut pane = Pane::new(
            "test-2".to_string(),
            "/bin/sleep".to_string(),
            vec!["60".to_string()],
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("Pane creation failed");

        // Feed some bytes through the vt100 parser
        pane.process_bytes(b"hello world\r\n");

        let screen = pane.screen();
        let contents = screen.contents();
        assert!(
            contents.contains("hello world"),
            "Screen should contain 'hello world', got: {contents}"
        );

        let _ = pane.kill();
    }

    #[test]
    fn test_pane_read_output_through_vt100() {
        let pane = Pane::new(
            "test-3".to_string(),
            "/bin/echo".to_string(),
            vec!["vt100-test".to_string()],
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("Pane creation failed");

        let reader = pane.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("vt100-test"),
            "Expected 'vt100-test' in: {text}"
        );
    }

    #[test]
    fn test_pane_write_input() {
        let pane = Pane::new(
            "test-4".to_string(),
            "/bin/cat".to_string(),
            vec![],
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("Pane creation failed");

        let reader = pane.reader().expect("reader failed");
        pane.write_input(b"pane-input\n").expect("write failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("pane-input"),
            "Expected 'pane-input' in: {text}"
        );
    }

    #[test]
    fn test_pane_resize() {
        let pane = Pane::new(
            "test-5".to_string(),
            "/bin/sleep".to_string(),
            vec!["60".to_string()],
            80,
            24,
            HashMap::new(),
            None,
        );

        // PTY resources may be exhausted from parallel tests; skip if so
        let mut pane = match pane {
            Ok(p) => p,
            Err(_) => return,
        };

        pane.resize(120, 48).expect("resize should succeed");

        // vt100 parser should reflect new size
        let screen = pane.screen();
        assert_eq!(screen.size(), (48, 120));

        let _ = pane.kill();
    }

    #[test]
    fn test_pane_check_status_completed() {
        let mut pane = Pane::new(
            "test-6".to_string(),
            "/usr/bin/true".to_string(),
            vec![],
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("Pane creation failed");

        assert_eq!(pane.status(), &PaneStatus::Running);

        let mut completed = false;
        for _ in 0..50 {
            if let Ok(status) = pane.check_status() {
                if *status != PaneStatus::Running {
                    completed = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        assert!(completed, "Process should have completed");
        assert_eq!(pane.status(), &PaneStatus::Completed(0));
    }

    #[test]
    fn test_pane_mark_error() {
        let mut pane = Pane::new(
            "test-7".to_string(),
            "/bin/sleep".to_string(),
            vec!["60".to_string()],
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("Pane creation failed");

        pane.mark_error("test error");
        assert_eq!(pane.status(), &PaneStatus::Error("test error".to_string()));

        let _ = pane.kill();
    }

    #[test]
    fn test_pane_kill() {
        let pane = Pane::new(
            "test-8".to_string(),
            "/bin/sleep".to_string(),
            vec!["60".to_string()],
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("Pane creation failed");

        pane.kill().expect("kill should succeed");
    }

    #[test]
    fn test_pane_status_enum() {
        let running = PaneStatus::Running;
        let completed = PaneStatus::Completed(0);
        let error = PaneStatus::Error("fail".to_string());

        assert_eq!(running, PaneStatus::Running);
        assert_eq!(completed, PaneStatus::Completed(0));
        assert_ne!(PaneStatus::Completed(0), PaneStatus::Completed(1));
        assert_eq!(error, PaneStatus::Error("fail".to_string()));
    }
}
