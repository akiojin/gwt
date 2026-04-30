//! Cross-platform PTY handle: spawn, I/O, resize, kill.

use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(not(windows))]
use std::path::Path;

use portable_pty::{native_pty_system, CommandBuilder, ExitStatus, MasterPty, PtySize};
use tracing::instrument;

use crate::TerminalError;

mod process_group;
#[cfg(windows)]
mod windows_spawn;

use process_group::ProcessGroup;

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
/// Dropping a `PtyHandle` terminates the child and any descendants that were
/// attached to its process group / Job Object.
pub struct PtyHandle {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    // Wrapped so `kill` (which takes `&self`) can synchronously terminate the
    // group without waiting for `Drop`. Declared last so that when `Drop` runs
    // the direct child has already been signaled above.
    process_group: Mutex<ProcessGroup>,
}

impl PtyHandle {
    /// Spawn a child process with a PTY.
    #[instrument(skip_all, fields(cmd = %config.command))]
    pub fn spawn(config: SpawnConfig) -> Result<Self, TerminalError> {
        let config = normalize_spawn_config(config);
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

        let process_group = child
            .process_id()
            .map(ProcessGroup::attach)
            .unwrap_or_default();

        Ok(Self {
            master: Arc::new(Mutex::new(pair.master)),
            child: Arc::new(Mutex::new(child)),
            writer: Arc::new(Mutex::new(writer)),
            process_group: Mutex::new(process_group),
        })
    }

    /// Send bytes to the PTY stdin.
    pub fn write_input(&self, data: &[u8]) -> Result<(), TerminalError> {
        let data_len = data.len();
        let lock_started = Instant::now();
        let mut writer = self.writer.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        let lock_wait_us = lock_started.elapsed().as_micros() as u64;

        let write_started = Instant::now();
        let write_result = writer.write_all(data);
        let write_us = write_started.elapsed().as_micros() as u64;
        write_result.map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })?;

        let flush_started = Instant::now();
        let flush_result = writer.flush();
        let flush_us = flush_started.elapsed().as_micros() as u64;

        tracing::debug!(
            target: "gwt_input_trace",
            stage = "pty_writer",
            data_len,
            lock_wait_us,
            write_us,
            flush_us,
            ok = flush_result.is_ok(),
            "PTY writer completed write_all + flush"
        );

        flush_result.map_err(|e| TerminalError::PtyIoError {
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

    /// Terminate the child process and every descendant in its process group.
    ///
    /// Terminating the group is required so that grandchildren cannot keep
    /// the PTY slave open after the direct child exits. While the slave stays
    /// open the master reader does not observe EOF, which would otherwise
    /// strand the reader thread (and its `Arc<Mutex<Pane>>`) and prevent the
    /// Drop chain from running.
    pub fn kill(&self) -> Result<(), TerminalError> {
        let mut child = self.child.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        let kill_result = child.kill();
        drop(child);

        // Always sweep descendants, even if the direct kill failed: the group
        // terminate is idempotent and uses an independent kernel path.
        let mut group = match self.process_group.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        group.terminate();
        drop(group);

        kill_result.map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })
    }

    /// Returns the OS process id of the spawned child, if available.
    pub fn process_id(&self) -> Option<u32> {
        self.child.lock().ok().and_then(|c| c.process_id())
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

fn normalize_spawn_config(config: SpawnConfig) -> SpawnConfig {
    #[cfg(windows)]
    {
        windows_spawn::normalize_spawn_config(config)
    }

    #[cfg(not(windows))]
    {
        normalize_non_windows_spawn_config(config)
    }
}

#[cfg(not(windows))]
fn normalize_non_windows_spawn_config(mut config: SpawnConfig) -> SpawnConfig {
    if let Some(command) = resolve_command_from_env_path(&config.command, &config.env) {
        config.command = command.display().to_string();
    }
    config
}

#[cfg(not(windows))]
fn resolve_command_from_env_path(command: &str, env: &HashMap<String, String>) -> Option<PathBuf> {
    if command.is_empty() || command.contains('/') {
        return None;
    }
    let path_value = env.get("PATH")?;
    std::env::split_paths(path_value).find_map(|dir| {
        let candidate = dir.join(command);
        is_executable_file(&candidate).then_some(candidate)
    })
}

#[cfg(all(not(windows), unix))]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(all(not(windows), not(unix)))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

impl Drop for PtyHandle {
    fn drop(&mut self) {
        // Best-effort termination: must never panic from Drop and must not
        // block the caller for long. Tolerate poisoned mutexes.
        let mut guard = match self.child.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let _ = guard.kill();

        // Short reap loop so subsequent try_wait callers observe the exit.
        // Capped at ~500ms so Drop never stalls the UI thread.
        for _ in 0..20 {
            match guard.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
        drop(guard);

        // Belt-and-suspenders: explicitly terminate the group in case `kill`
        // was never called (e.g. the handle was dropped without going through
        // stop_window_runtime). ProcessGroup::terminate is idempotent.
        if let Ok(mut group) = self.process_group.lock() {
            group.terminate();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::test_util::{
        answer_cursor_position_query, echo_command, env_command, lock_pty_test, pwd_command,
        read_until_contains, read_with_timeout, sleep_command, stdin_echo_command, success_command,
        TestCommand,
    };

    fn command_config(command: TestCommand) -> SpawnConfig {
        SpawnConfig {
            command: command.command,
            args: command.args,
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        }
    }

    fn echo_config(msg: &str) -> SpawnConfig {
        command_config(echo_command(msg))
    }

    fn sleep_config(secs: &str) -> SpawnConfig {
        command_config(sleep_command(secs))
    }

    #[cfg(unix)]
    #[test]
    fn normalize_spawn_config_resolves_command_from_config_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let runner = dir.path().join("gwt-test-runner");
        std::fs::write(&runner, "#!/bin/sh\nexit 0\n").expect("write runner");
        let mut permissions = std::fs::metadata(&runner)
            .expect("runner metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&runner, permissions).expect("chmod runner");

        let config = SpawnConfig {
            command: "gwt-test-runner".to_string(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            env: HashMap::from([("PATH".to_string(), dir.path().display().to_string())]),
            remove_env: Vec::new(),
            cwd: None,
        };

        let normalized = normalize_spawn_config(config);

        assert_eq!(PathBuf::from(normalized.command), runner);
    }

    #[test]
    fn test_spawn_and_read_output() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(echo_config("hello")).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("hello"), "Expected 'hello' in: {text}");
    }

    #[test]
    fn test_write_input() {
        let _pty_guard = lock_pty_test();
        let config = command_config(stdin_echo_command());
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("reader failed");
        handle.write_input(b"test-input\n").expect("write failed");
        let output =
            read_until_contains(reader, Duration::from_secs(5), "test-input").expect("read failed");
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
        answer_cursor_position_query(&handle);
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
        let command = env_command();
        let config = SpawnConfig {
            command: command.command,
            args: command.args,
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        };
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        answer_cursor_position_query(&handle);
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
        let command = pwd_command();
        let config = SpawnConfig {
            command: command.command,
            args: command.args,
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(temp.clone()),
        };
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output).trim().to_string();
        // The output should be the canonical path of the temp dir.
        // On macOS, /tmp -> /private/tmp or /var -> /private/var.
        let canonical_temp = std::fs::canonicalize(&temp)
            .unwrap_or(temp)
            .display()
            .to_string();
        let canonical_temp = canonical_temp
            .strip_prefix(r"\\?\")
            .unwrap_or(&canonical_temp)
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
        let command = env_command();
        let config = SpawnConfig {
            command: command.command,
            args: command.args,
            cols: 80,
            rows: 24,
            env,
            remove_env: vec!["HOME".to_string()],
            cwd: None,
        };
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        answer_cursor_position_query(&handle);
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

    #[test]
    fn test_success_command_exits_zero() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(command_config(success_command())).expect("spawn failed");
        answer_cursor_position_query(&handle);
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
}
