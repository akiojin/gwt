//! Cross-platform PTY handle: spawn, I/O, resize, kill.

use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use portable_pty::{native_pty_system, CommandBuilder, ExitStatus, MasterPty, PtySize};
use tracing::instrument;

use crate::TerminalError;

/// Configuration for spawning a PTY process.
pub struct SpawnConfig {
    /// Command to execute.
    pub command: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Initial terminal size.
    pub cols: u16,
    /// Initial terminal rows.
    pub rows: u16,
    /// Environment variables to set.
    pub env: HashMap<String, String>,
    /// Inherited environment variable names to remove before applying `env`.
    pub remove_env: Vec<String>,
    /// Working directory.
    pub cwd: Option<PathBuf>,
}

/// Handle to a spawned PTY process.
///
/// Provides methods for I/O, resize, and process lifecycle management.
pub struct PtyHandle {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl PtyHandle {
    /// Spawn a child process with a PTY.
    #[instrument(skip_all, fields(cmd = %config.command))]
    pub fn spawn(config: SpawnConfig) -> Result<Self, TerminalError> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: config.rows,
                cols: config.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::PtyCreationFailed {
                reason: e.to_string(),
            })?;

        let mut cmd = CommandBuilder::new(&config.command);
        cmd.args(&config.args);
        if let Some(ref cwd) = config.cwd {
            cmd.cwd(cwd);
        }
        for key in &config.remove_env {
            cmd.env_remove(key);
        }
        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        let child =
            pair.slave
                .spawn_command(cmd)
                .map_err(|e| TerminalError::PtyCreationFailed {
                    reason: e.to_string(),
                })?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| TerminalError::PtyCreationFailed {
                reason: format!("take_writer: {e}"),
            })?;

        Ok(Self {
            master: Arc::new(Mutex::new(pair.master)),
            child: Arc::new(Mutex::new(child)),
            writer: Arc::new(Mutex::new(writer)),
        })
    }

    /// Send bytes to the PTY stdin.
    pub fn write_input(&self, data: &[u8]) -> Result<(), TerminalError> {
        let mut writer = self.writer.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        writer
            .write_all(data)
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })?;
        writer.flush().map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })?;
        Ok(())
    }

    /// Resize the PTY window.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), TerminalError> {
        let master = self.master.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        master
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

    /// Terminate the child process.
    pub fn kill(&self) -> Result<(), TerminalError> {
        let mut child = self.child.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        child.kill().map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })
    }

    /// Returns a reader for the PTY output.
    ///
    /// The reader can be used in a separate thread/task to read output asynchronously.
    pub fn reader(&self) -> Result<Box<dyn Read + Send>, TerminalError> {
        let master = self.master.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        master
            .try_clone_reader()
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })
    }

    /// Try to wait for the child process without blocking.
    ///
    /// Returns `Some(ExitStatus)` if the child has exited, `None` if still running.
    pub fn try_wait(&self) -> Result<Option<ExitStatus>, TerminalError> {
        let mut child = self.child.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        child.try_wait().map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{lock_pty_test, read_with_timeout};
    use std::time::Duration;

    fn echo_config(msg: &str) -> SpawnConfig {
        SpawnConfig {
            command: "/bin/echo".to_string(),
            args: vec![msg.to_string()],
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        }
    }

    fn sleep_config(secs: &str) -> SpawnConfig {
        SpawnConfig {
            command: "/bin/sleep".to_string(),
            args: vec![secs.to_string()],
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        }
    }

    #[test]
    fn test_spawn_and_read_output() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(echo_config("hello")).expect("spawn failed");
        let reader = handle.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("hello"), "Expected 'hello' in: {text}");
    }

    #[test]
    fn test_write_input() {
        let _pty_guard = lock_pty_test();
        let config = SpawnConfig {
            command: "/bin/cat".to_string(),
            args: vec![],
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        };
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        let reader = handle.reader().expect("reader failed");
        handle.write_input(b"test-input\n").expect("write failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("test-input"),
            "Expected 'test-input' in: {text}"
        );
    }

    #[test]
    fn test_resize() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(sleep_config("1")).expect("spawn failed");
        handle.resize(120, 48).expect("resize should succeed");
    }

    #[test]
    fn test_kill() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(sleep_config("60")).expect("spawn failed");
        handle.kill().expect("kill should succeed");

        let mut exited = false;
        for _ in 0..50 {
            if let Ok(Some(_)) = handle.try_wait() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(exited, "Process should have exited after kill");
    }

    #[test]
    fn test_try_wait_running() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(sleep_config("60")).expect("spawn failed");
        let result = handle.try_wait().expect("try_wait failed");
        assert!(result.is_none(), "Process should still be running");
        handle.kill().ok();
    }

    #[test]
    fn test_try_wait_completed() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(echo_config("done")).expect("spawn failed");
        let mut exited = false;
        for _ in 0..50 {
            if let Ok(Some(status)) = handle.try_wait() {
                assert!(status.success());
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(exited, "Process should have completed");
    }

    #[test]
    fn test_spawn_with_env() {
        let _pty_guard = lock_pty_test();
        let mut env = HashMap::new();
        env.insert("GWT_TEST_VAR".to_string(), "test_value".to_string());
        let config = SpawnConfig {
            command: "/usr/bin/env".to_string(),
            args: vec![],
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        };
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        let reader = handle.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("GWT_TEST_VAR=test_value"),
            "Expected env var in: {text}"
        );
    }

    #[test]
    fn test_spawn_with_cwd() {
        let _pty_guard = lock_pty_test();
        let temp = std::env::temp_dir();
        let config = SpawnConfig {
            command: "/bin/pwd".to_string(),
            args: vec![],
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(temp.clone()),
        };
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        let reader = handle.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output).trim().to_string();
        // The output should be the canonical path of the temp dir.
        // On macOS, /tmp -> /private/tmp or /var -> /private/var.
        let canonical_temp = std::fs::canonicalize(&temp)
            .unwrap_or(temp)
            .display()
            .to_string();
        assert!(
            text.contains(&canonical_temp) || canonical_temp.contains(text.trim()),
            "Expected temp dir path in output.\n  output: {text}\n  expected: {canonical_temp}"
        );
    }

    #[test]
    fn test_spawn_invalid_command_fails() {
        let _pty_guard = lock_pty_test();
        let config = SpawnConfig {
            command: "/nonexistent/binary".to_string(),
            args: vec![],
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        };
        let result = PtyHandle::spawn(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_spawn_with_removed_inherited_env() {
        let _pty_guard = lock_pty_test();
        let mut env = HashMap::new();
        env.insert("GWT_REMOVE_CHECK".to_string(), "expected".to_string());
        let config = SpawnConfig {
            command: "/usr/bin/env".to_string(),
            args: vec![],
            cols: 80,
            rows: 24,
            env,
            remove_env: vec!["HOME".to_string()],
            cwd: None,
        };
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        let reader = handle.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("GWT_REMOVE_CHECK=expected"),
            "Expected env var in: {text}"
        );
        assert!(
            !text.lines().any(|line| line.starts_with("HOME=")),
            "Expected inherited HOME to be removed from: {text}"
        );
    }
}
