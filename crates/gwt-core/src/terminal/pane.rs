//! Terminal pane: integrates PTY, emulator, and scrollback

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::emulator::TerminalEmulator;
use super::pty::{PtyConfig, PtyHandle};
use super::renderer::render_to_buffer;
use super::scrollback::ScrollbackFile;
use super::TerminalError;

/// Status of a terminal pane's child process.
#[derive(Debug, Clone, PartialEq)]
pub enum PaneStatus {
    Running,
    Completed(i32),
    Error(String),
}

/// Configuration for creating a new terminal pane.
pub struct PaneConfig {
    pub pane_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub branch_name: String,
    pub agent_name: String,
    pub agent_color: Color,
    pub rows: u16,
    pub cols: u16,
    pub env_vars: HashMap<String, String>,
}

/// A terminal pane integrating PTY, VT100 emulator, and scrollback.
pub struct TerminalPane {
    pane_id: String,
    emulator: TerminalEmulator,
    scrollback: ScrollbackFile,
    pty: PtyHandle,
    writer: Option<Box<dyn Write + Send>>,
    branch_name: String,
    agent_name: String,
    agent_color: Color,
    status: PaneStatus,
    started_at: chrono::DateTime<chrono::Utc>,
}

impl TerminalPane {
    /// Create a new terminal pane from the given configuration.
    pub fn new(config: PaneConfig) -> Result<Self, TerminalError> {
        let mut env_vars = config.env_vars;
        env_vars.insert("GWT_PANE_ID".to_string(), config.pane_id.clone());
        env_vars.insert("GWT_BRANCH".to_string(), config.branch_name.clone());
        env_vars.insert("GWT_AGENT".to_string(), config.agent_name.clone());

        let pty_config = PtyConfig {
            command: config.command,
            args: config.args,
            working_dir: config.working_dir,
            env_vars,
            rows: config.rows,
            cols: config.cols,
        };

        let pty = PtyHandle::new(pty_config)?;
        let writer = Some(pty.take_writer()?);
        let emulator = TerminalEmulator::new(config.rows, config.cols);
        let scrollback = ScrollbackFile::new(&config.pane_id)?;

        Ok(Self {
            pane_id: config.pane_id,
            emulator,
            scrollback,
            pty,
            writer,
            branch_name: config.branch_name,
            agent_name: config.agent_name,
            agent_color: config.agent_color,
            status: PaneStatus::Running,
            started_at: chrono::Utc::now(),
        })
    }

    /// Take the PTY reader for external I/O loop management.
    pub fn take_reader(&self) -> Result<Box<dyn std::io::Read + Send>, TerminalError> {
        self.pty.take_reader()
    }

    /// Process bytes from the PTY output through the emulator and scrollback.
    pub fn process_bytes(&mut self, bytes: &[u8]) -> Result<(), TerminalError> {
        self.emulator.process(bytes);
        self.scrollback.write(bytes)?;
        Ok(())
    }

    /// Write input data to the PTY.
    pub fn write_input(&mut self, data: &[u8]) -> Result<(), TerminalError> {
        if let Some(ref mut writer) = self.writer {
            writer
                .write_all(data)
                .map_err(|e| TerminalError::PtyIoError {
                    details: e.to_string(),
                })?;
            writer.flush().map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })?;
        } else {
            return Err(TerminalError::PtyIoError {
                details: "writer not available".to_string(),
            });
        }
        Ok(())
    }

    /// Resize the terminal pane (emulator + PTY).
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), TerminalError> {
        self.emulator.resize(rows, cols);
        self.pty.resize(rows, cols)
    }

    /// Check and update the pane's process status.
    pub fn check_status(&mut self) -> Result<&PaneStatus, TerminalError> {
        if self.status == PaneStatus::Running {
            if let Some(exit_status) = self.pty.try_wait()? {
                if exit_status.success() {
                    self.status = PaneStatus::Completed(0);
                } else {
                    // portable-pty ExitStatus doesn't expose the code directly on all platforms,
                    // but we can check success/failure
                    self.status = PaneStatus::Completed(1);
                }
            }
        }
        Ok(&self.status)
    }

    /// Get a reference to the VT100 screen.
    pub fn screen(&self) -> &vt100::Screen {
        self.emulator.screen()
    }

    /// Render the terminal contents into a ratatui buffer.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        render_to_buffer(self.emulator.screen(), area, buf);
    }

    /// Get the pane ID.
    pub fn pane_id(&self) -> &str {
        &self.pane_id
    }

    /// Get the branch name.
    pub fn branch_name(&self) -> &str {
        &self.branch_name
    }

    /// Get the agent name.
    pub fn agent_name(&self) -> &str {
        &self.agent_name
    }

    /// Get the agent color.
    pub fn agent_color(&self) -> Color {
        self.agent_color
    }

    /// Get the current status.
    pub fn status(&self) -> &PaneStatus {
        &self.status
    }

    /// Get the start time.
    pub fn started_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.started_at
    }

    /// Kill the child process.
    pub fn kill(&mut self) -> Result<(), TerminalError> {
        self.pty.kill()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Helper: read from PTY reader in a separate thread with timeout.
    fn read_with_timeout(
        mut reader: Box<dyn std::io::Read + Send>,
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 4096];
            let mut output = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        output.extend_from_slice(&buf[..n]);
                        let _ = tx.send(Ok(output.clone()));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        break;
                    }
                }
            }
        });

        let mut last_output = Vec::new();
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(data)) => last_output = data,
                Ok(Err(e)) => return Err(e),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !last_output.is_empty() {
                        return Ok(last_output);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        if last_output.is_empty() {
            Err("Timed out with no output".to_string())
        } else {
            Ok(last_output)
        }
    }

    fn make_config(command: &str, args: Vec<&str>) -> PaneConfig {
        PaneConfig {
            pane_id: format!("test-pane-{}", std::process::id()),
            command: command.to_string(),
            args: args.into_iter().map(|s| s.to_string()).collect(),
            working_dir: std::env::temp_dir(),
            branch_name: "feature/test".to_string(),
            agent_name: "test-agent".to_string(),
            agent_color: Color::Green,
            rows: 24,
            cols: 80,
            env_vars: HashMap::new(),
        }
    }

    // 1. PaneStatus enum test
    #[test]
    fn test_pane_status_values() {
        let running = PaneStatus::Running;
        let completed = PaneStatus::Completed(0);
        let error = PaneStatus::Error("fail".to_string());

        assert_eq!(running, PaneStatus::Running);
        assert_eq!(completed, PaneStatus::Completed(0));
        assert_ne!(PaneStatus::Completed(0), PaneStatus::Completed(1));
        assert_eq!(error, PaneStatus::Error("fail".to_string()));
    }

    // 2. Pane creation and output processing test
    #[test]
    fn test_pane_creation_and_process_bytes() {
        let config = make_config("/bin/echo", vec!["hello"]);
        let mut pane = TerminalPane::new(config).expect("Failed to create pane");

        let reader = pane.take_reader().expect("Failed to get reader");
        let output =
            read_with_timeout(reader, Duration::from_secs(5)).expect("Failed to read output");

        pane.process_bytes(&output)
            .expect("Failed to process bytes");

        let cell = pane.screen().cell(0, 0).expect("cell(0,0) should exist");
        assert_eq!(cell.contents(), "h", "Expected 'h' at (0,0)");
    }

    // 3. Input writing test
    #[test]
    fn test_write_input() {
        let config = make_config("/bin/cat", vec![]);
        let mut pane = TerminalPane::new(config).expect("Failed to create pane");

        let reader = pane.take_reader().expect("Failed to get reader");

        pane.write_input(b"test\n").expect("Failed to write input");

        let output =
            read_with_timeout(reader, Duration::from_secs(5)).expect("Failed to read output");

        pane.process_bytes(&output)
            .expect("Failed to process bytes");

        let output_str = String::from_utf8_lossy(&output);
        assert!(
            output_str.contains("test"),
            "Expected 'test' in output, got: {output_str}"
        );
    }

    // 4. Resize test
    #[test]
    fn test_resize() {
        let config = make_config("/bin/sleep", vec!["1"]);
        let mut pane = TerminalPane::new(config).expect("Failed to create pane");

        pane.resize(48, 120).expect("Resize should succeed");
        assert_eq!(pane.emulator.size(), (48, 120));
    }

    // 5. Status test (process completes with exit code 0)
    #[test]
    fn test_check_status_completed() {
        let config = make_config("/usr/bin/true", vec![]);
        let mut pane = TerminalPane::new(config).expect("Failed to create pane");

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

    // 6. Accessor tests
    #[test]
    fn test_accessors() {
        let config = make_config("/bin/sleep", vec!["1"]);
        let pane = TerminalPane::new(config).expect("Failed to create pane");

        assert!(pane.pane_id().starts_with("test-pane-"));
        assert_eq!(pane.branch_name(), "feature/test");
        assert_eq!(pane.agent_name(), "test-agent");
        assert_eq!(pane.agent_color(), Color::Green);
        assert_eq!(pane.status(), &PaneStatus::Running);
        assert!(pane.started_at() <= chrono::Utc::now());
    }

    // 7. Kill test
    #[test]
    fn test_kill() {
        let config = make_config("/bin/sleep", vec!["60"]);
        let mut pane = TerminalPane::new(config).expect("Failed to create pane");

        pane.kill().expect("Kill should succeed");

        let mut exited = false;
        for _ in 0..50 {
            if let Ok(status) = pane.check_status() {
                if *status != PaneStatus::Running {
                    exited = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        assert!(exited, "Process should have exited after kill");
        assert_ne!(pane.status(), &PaneStatus::Running);
    }
}
