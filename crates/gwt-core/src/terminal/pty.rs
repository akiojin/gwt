//! PTY management module
//!
//! Manages pseudo-terminal creation, I/O, resize, and cleanup.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;

use portable_pty::{native_pty_system, CommandBuilder, ExitStatus, MasterPty, PtySize};

use super::TerminalError;

/// Configuration for creating a new PTY.
pub struct PtyConfig {
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub env_vars: HashMap<String, String>,
    pub rows: u16,
    pub cols: u16,
}

fn is_windows_batch_script(command: &str) -> bool {
    let lower = command.trim().to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
}

fn resolve_spawn_command(command: &str, args: &[String]) -> (String, Vec<String>) {
    if cfg!(windows) && is_windows_batch_script(command) {
        // On Windows, .cmd/.bat scripts must be launched through cmd.exe in PTY contexts.
        let mut wrapped_args = vec![
            "/d".to_string(),
            "/s".to_string(),
            "/c".to_string(),
            command.to_string(),
        ];
        wrapped_args.extend(args.iter().cloned());
        return ("cmd.exe".to_string(), wrapped_args);
    }

    (command.to_string(), args.to_vec())
}

/// Handle to a PTY instance with its child process.
pub struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyHandle {
    /// Create a new PTY, spawn the given command, and return a handle.
    pub fn new(config: PtyConfig) -> Result<Self, TerminalError> {
        let pty_system = native_pty_system();

        let size = PtySize {
            rows: config.rows,
            cols: config.cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(size)
            .map_err(|e| TerminalError::PtyCreationFailed {
                reason: e.to_string(),
            })?;

        let (spawn_command, spawn_args) = resolve_spawn_command(&config.command, &config.args);

        let mut cmd = CommandBuilder::new(&spawn_command);
        for arg in &spawn_args {
            cmd.arg(arg);
        }
        cmd.cwd(&config.working_dir);

        // Default to a color-capable terminal environment.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        // Set user-provided environment variables
        for (key, value) in &config.env_vars {
            cmd.env(key, value);
        }

        let child =
            pair.slave
                .spawn_command(cmd)
                .map_err(|e| TerminalError::PtyCreationFailed {
                    reason: e.to_string(),
                })?;

        // Drop slave after spawning (required by portable-pty)
        drop(pair.slave);

        Ok(Self {
            master: pair.master,
            child,
        })
    }

    /// Get a cloneable reader for PTY output.
    pub fn take_reader(&self) -> Result<Box<dyn Read + Send>, TerminalError> {
        self.master
            .try_clone_reader()
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })
    }

    /// Take the single writer for PTY input.
    pub fn take_writer(&self) -> Result<Box<dyn Write + Send>, TerminalError> {
        self.master
            .take_writer()
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })
    }

    /// Resize the PTY to the given dimensions.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), TerminalError> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })
    }

    /// Non-blocking check if the child process has exited.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, TerminalError> {
        self.child
            .try_wait()
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })
    }

    /// Kill the child process.
    pub fn kill(&mut self) -> Result<(), TerminalError> {
        self.child.kill().map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn is_windows_batch_script_detects_cmd_and_bat() {
        assert!(is_windows_batch_script("npx.cmd"));
        assert!(is_windows_batch_script("C:\\Tools\\runner.BAT"));
        assert!(!is_windows_batch_script("codex"));
        assert!(!is_windows_batch_script("/usr/bin/codex"));
    }

    #[test]
    fn resolve_spawn_command_wraps_batch_script_only_on_windows() {
        let args = vec!["--yes".to_string(), "@openai/codex@latest".to_string()];
        let (program, resolved_args) = resolve_spawn_command("C:\\Tools\\npx.cmd", &args);

        if cfg!(windows) {
            assert_eq!(program, "cmd.exe");
            assert_eq!(
                resolved_args,
                vec![
                    "/d".to_string(),
                    "/s".to_string(),
                    "/c".to_string(),
                    "C:\\Tools\\npx.cmd".to_string(),
                    "--yes".to_string(),
                    "@openai/codex@latest".to_string()
                ]
            );
        } else {
            assert_eq!(program, "C:\\Tools\\npx.cmd");
            assert_eq!(resolved_args, args);
        }
    }

    #[test]
    fn resolve_spawn_command_keeps_non_batch_command() {
        let args = vec!["--version".to_string()];
        let (program, resolved_args) = resolve_spawn_command("codex", &args);
        assert_eq!(program, "codex");
        assert_eq!(resolved_args, args);
    }

    /// Helper: read from PTY reader in a separate thread with timeout.
    fn read_with_timeout(
        mut reader: Box<dyn Read + Send>,
        timeout: Duration,
    ) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 4096];
            let mut output = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        output.extend_from_slice(&buf[..n]);
                        let _ = tx.send(Ok(String::from_utf8_lossy(&output).to_string()));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        break;
                    }
                }
            }
        });

        // Collect output until timeout
        let mut last_output = String::new();
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(s)) => last_output = s,
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

    #[test]
    fn test_pty_creation_and_echo() {
        let config = PtyConfig {
            command: "/bin/echo".to_string(),
            args: vec!["hello".to_string()],
            working_dir: std::env::temp_dir(),
            env_vars: HashMap::new(),
            rows: 24,
            cols: 80,
        };

        let handle = PtyHandle::new(config).expect("Failed to create PTY");
        let reader = handle.take_reader().expect("Failed to get reader");

        let output =
            read_with_timeout(reader, Duration::from_secs(5)).expect("Failed to read PTY output");

        assert!(
            output.contains("hello"),
            "Expected 'hello' in output, got: {output}"
        );
    }

    #[test]
    fn test_env_vars_set() {
        let mut env_vars = HashMap::new();
        env_vars.insert("GWT_PANE_ID".to_string(), "pane-42".to_string());
        env_vars.insert("GWT_BRANCH".to_string(), "feature/test".to_string());
        env_vars.insert("GWT_SESSION_ID".to_string(), "sess-001".to_string());

        let config = PtyConfig {
            command: "/usr/bin/env".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            env_vars,
            rows: 24,
            cols: 80,
        };

        let handle = PtyHandle::new(config).expect("Failed to create PTY");
        let reader = handle.take_reader().expect("Failed to get reader");

        let output =
            read_with_timeout(reader, Duration::from_secs(5)).expect("Failed to read PTY output");

        assert!(
            output.contains("GWT_PANE_ID=pane-42"),
            "Expected GWT_PANE_ID in output, got: {output}"
        );
        assert!(
            output.contains("GWT_BRANCH=feature/test"),
            "Expected GWT_BRANCH in output, got: {output}"
        );
        assert!(
            output.contains("GWT_SESSION_ID=sess-001"),
            "Expected GWT_SESSION_ID in output, got: {output}"
        );
        assert!(
            output.contains("TERM=xterm-256color"),
            "Expected TERM=xterm-256color in output, got: {output}"
        );
        assert!(
            output.contains("COLORTERM=truecolor"),
            "Expected COLORTERM=truecolor in output, got: {output}"
        );
    }

    #[test]
    fn test_resize() {
        let config = PtyConfig {
            command: "/bin/sleep".to_string(),
            args: vec!["1".to_string()],
            working_dir: std::env::temp_dir(),
            env_vars: HashMap::new(),
            rows: 24,
            cols: 80,
        };

        let handle = PtyHandle::new(config).expect("Failed to create PTY");
        let result = handle.resize(48, 120);
        assert!(result.is_ok(), "Resize should succeed: {result:?}");
    }

    #[test]
    fn test_process_exit_detection() {
        let config = PtyConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            env_vars: HashMap::new(),
            rows: 24,
            cols: 80,
        };

        let mut handle = PtyHandle::new(config).expect("Failed to create PTY");

        // Wait for process to exit
        let mut exited = false;
        for _ in 0..50 {
            if let Ok(Some(_status)) = handle.try_wait() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        assert!(exited, "Process should have exited");
    }

    #[test]
    fn test_invalid_command_error() {
        let config = PtyConfig {
            command: "/nonexistent/command/that/does/not/exist".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            env_vars: HashMap::new(),
            rows: 24,
            cols: 80,
        };

        let result = PtyHandle::new(config);
        assert!(
            result.is_err(),
            "Creating PTY with invalid command should fail"
        );

        if let Err(TerminalError::PtyCreationFailed { reason }) = result {
            assert!(
                !reason.is_empty(),
                "Error reason should not be empty: {reason}"
            );
        } else {
            panic!("Expected TerminalError::PtyCreationFailed");
        }
    }
}
