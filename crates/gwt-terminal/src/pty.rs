//! Cross-platform PTY handle: spawn, I/O, resize, kill.

use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::path::Path;

use portable_pty::{native_pty_system, CommandBuilder, ExitStatus, MasterPty, PtySize};
use tracing::instrument;

use crate::TerminalError;

mod process_group;
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
        normalize_windows_spawn_config(config)
    }

    #[cfg(not(windows))]
    {
        config
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowsSpawnTarget {
    command: String,
    args_prefix: Vec<String>,
}

#[cfg(windows)]
fn normalize_windows_spawn_config(mut config: SpawnConfig) -> SpawnConfig {
    let resolved = resolve_windows_spawn_target(&config.command, &config.env, &config.remove_env)
        .unwrap_or_else(|| WindowsSpawnTarget {
            command: config.command.clone(),
            args_prefix: Vec::new(),
        });

    match windows_spawn_wrapper(
        Path::new(&resolved.command),
        &config.env,
        &config.remove_env,
    ) {
        Some((command, mut args)) => {
            args.extend(config.args);
            config.command = command;
            config.args = args;
        }
        None => {
            let mut args = resolved.args_prefix;
            args.extend(config.args);
            config.command = resolved.command;
            config.args = args;
        }
    }

    config
}

#[cfg(windows)]
fn resolve_windows_spawn_target(
    command: &str,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<WindowsSpawnTarget> {
    let command_path = Path::new(command);
    let has_separator = command_path
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty());

    if has_separator || command_path.is_absolute() {
        return resolve_windows_path_candidate(command_path, env, remove_env);
    }

    let path_value = windows_env_value("PATH", env, remove_env)?;
    for dir in std::env::split_paths(&path_value) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(command_path);
        if let Some(resolved) = resolve_windows_path_candidate(&candidate, env, remove_env) {
            return Some(resolved);
        }
    }

    resolve_windows_path_candidate(command_path, env, remove_env)
}

#[cfg(windows)]
fn resolve_windows_path_candidate(
    candidate: &Path,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<WindowsSpawnTarget> {
    if has_executable_extension(candidate) && candidate.exists() {
        return Some(WindowsSpawnTarget {
            command: candidate.display().to_string(),
            args_prefix: Vec::new(),
        });
    }

    if candidate.extension().is_none() {
        if candidate.exists() {
            if let Some(target) = parse_windows_npm_shim(candidate) {
                return Some(target);
            }
        }
        for ext in windows_path_extensions(env, remove_env) {
            let with_ext = candidate.with_extension(ext.trim_start_matches('.'));
            if let Some(target) = resolve_windows_existing_path(&with_ext) {
                return Some(target);
            }
        }
    }

    resolve_windows_existing_path(candidate)
}

#[cfg(windows)]
fn windows_spawn_wrapper(
    resolved: &Path,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<(String, Vec<String>)> {
    let ext = resolved
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())?;
    if ext != "cmd" && ext != "bat" {
        return None;
    }

    let comspec = windows_env_value("ComSpec", env, remove_env)
        .unwrap_or_else(|| std::ffi::OsString::from("cmd.exe"));

    // SPEC-1921 FR-082: Do NOT pass `/s`. `/s` forces CMD to strip the
    // quotes that surround the executable path, which breaks invocations
    // with whitespace in the path (e.g. `C:\Program Files\nodejs\npx.cmd`).
    // Without `/s`, CMD's default rule preserves the quotes when the
    // command line has the typical `"<exe>" <args>` shape we emit here.
    Some((
        PathBuf::from(comspec).display().to_string(),
        vec![
            "/d".to_string(),
            "/c".to_string(),
            resolved.display().to_string(),
        ],
    ))
}

#[cfg(windows)]
fn resolve_windows_existing_path(candidate: &Path) -> Option<WindowsSpawnTarget> {
    if !candidate.exists() {
        return None;
    }

    if let Some(target) = parse_windows_npm_shim(candidate) {
        return Some(target);
    }

    Some(WindowsSpawnTarget {
        command: candidate.display().to_string(),
        args_prefix: Vec::new(),
    })
}

#[cfg(windows)]
fn parse_windows_npm_shim(candidate: &Path) -> Option<WindowsSpawnTarget> {
    match candidate
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("cmd") | Some("bat") => parse_windows_cmd_shim(candidate),
        Some("exe") | Some("com") => None,
        _ => parse_windows_shell_shim(candidate),
    }
}

#[cfg(windows)]
fn parse_windows_shell_shim(candidate: &Path) -> Option<WindowsSpawnTarget> {
    let content = std::fs::read_to_string(candidate).ok()?;
    let base_dir = candidate.parent()?;
    let basedir_paths = collect_marker_paths(&content, "$basedir/");
    if basedir_paths.is_empty() {
        return None;
    }
    build_windows_shim_target(base_dir, &basedir_paths)
}

#[cfg(windows)]
fn parse_windows_cmd_shim(candidate: &Path) -> Option<WindowsSpawnTarget> {
    let content = std::fs::read_to_string(candidate).ok()?;
    let base_dir = candidate.parent()?;
    let dp0_paths = collect_marker_paths(&content, "%dp0%\\");
    if dp0_paths.is_empty() {
        return None;
    }
    build_windows_shim_target(base_dir, &dp0_paths)
}

#[cfg(windows)]
fn build_windows_shim_target(base_dir: &Path, raw_paths: &[String]) -> Option<WindowsSpawnTarget> {
    let executable = raw_paths.iter().find_map(|path| {
        let lower = path.to_ascii_lowercase();
        (lower.ends_with(".exe") || lower.ends_with(".com"))
            .then(|| base_dir.join(normalize_windows_rel_path(path)))
    });
    let script = raw_paths.iter().find_map(|path| {
        let lower = path.to_ascii_lowercase();
        (lower.ends_with(".js") || lower.ends_with(".cjs"))
            .then(|| base_dir.join(normalize_windows_rel_path(path)))
    });

    match (executable, script) {
        (Some(executable), Some(script)) if windows_is_node_runtime(&executable) => {
            let command = if executable.exists() {
                executable.display().to_string()
            } else {
                windows_local_node_command(base_dir)
            };
            Some(WindowsSpawnTarget {
                command,
                args_prefix: vec![script.display().to_string()],
            })
        }
        // SPEC-1921 FR-081: Node.js distribution shims (e.g.
        // `C:\Program Files\nodejs\npx`) reference `$basedir/node.exe` but
        // dereference the CLI script via a separate variable such as
        // `$CLI_BASEDIR`. Our marker scan never pairs node with a `.js`
        // script in that case. Substituting `node.exe` alone would drop the
        // script and pass the caller's agent args (`--yes @pkg@version ...`)
        // straight to node, yielding `bad option: --yes`. Refuse the
        // substitution so resolution falls back to the `.cmd` sibling.
        (Some(executable), None) if windows_is_node_runtime(&executable) => None,
        (Some(executable), _) if executable.exists() => Some(WindowsSpawnTarget {
            command: executable.display().to_string(),
            args_prefix: Vec::new(),
        }),
        (_, Some(script)) => Some(WindowsSpawnTarget {
            command: windows_local_node_command(base_dir),
            args_prefix: vec![script.display().to_string()],
        }),
        _ => None,
    }
}

#[cfg(windows)]
fn collect_marker_paths(content: &str, marker: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut remaining = content;
    while let Some(index) = remaining.find(marker) {
        let start = index + marker.len();
        let tail = &remaining[start..];
        let end = tail.find(['"', '\r', '\n']).unwrap_or(tail.len());
        let value = tail[..end].trim();
        if !value.is_empty() {
            values.push(value.to_string());
        }
        remaining = &tail[end..];
    }
    values
}

#[cfg(windows)]
fn normalize_windows_rel_path(value: &str) -> PathBuf {
    PathBuf::from(value.replace('/', "\\"))
}

#[cfg(windows)]
fn windows_local_node_command(base_dir: &Path) -> String {
    ["node.exe", "node"]
        .into_iter()
        .map(|name| base_dir.join(name))
        .find(|path| path.exists())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "node".to_string())
}

#[cfg(windows)]
fn windows_is_node_runtime(path: &Path) -> bool {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("node"))
        .unwrap_or(false)
}

#[cfg(windows)]
fn windows_env_value(
    key: &str,
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Option<std::ffi::OsString> {
    if let Some(value) = env
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, value)| std::ffi::OsString::from(value))
    {
        return Some(value);
    }

    if remove_env
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(key))
    {
        return None;
    }

    std::env::var_os(key)
}

#[cfg(windows)]
fn windows_path_extensions(env: &HashMap<String, String>, remove_env: &[String]) -> Vec<String> {
    let raw = windows_env_value("PATHEXT", env, remove_env)
        .and_then(|value| value.into_string().ok())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());

    raw.split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.to_ascii_lowercase())
        .collect()
}

#[cfg(windows)]
fn has_executable_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "exe" | "com"))
        .unwrap_or(false)
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
    use crate::test_util::{lock_pty_test, read_with_timeout};

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

    #[cfg(windows)]
    #[test]
    fn windows_wraps_cmd_shims_with_comspec() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let shim = bin_dir.join("claude");
        let cmd = bin_dir.join("claude.cmd");
        std::fs::write(&shim, "#!/bin/sh\n").expect("shim");
        std::fs::write(&cmd, "@echo off\r\n").expect("cmd");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert(
            "PATHEXT".to_string(),
            ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC".to_string(),
        );
        env.insert(
            "ComSpec".to_string(),
            r"C:\Windows\System32\cmd.exe".to_string(),
        );

        let normalized = normalize_windows_spawn_config(SpawnConfig {
            command: "claude".to_string(),
            args: vec!["--dangerously-skip-permissions".to_string()],
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        });

        assert_eq!(normalized.command, r"C:\Windows\System32\cmd.exe");
        assert_eq!(
            normalized.args,
            vec![
                "/d".to_string(),
                "/c".to_string(),
                cmd.display().to_string(),
                "--dangerously-skip-permissions".to_string(),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefers_real_exe_before_extensionless_shim() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let shim = bin_dir.join("codex");
        let exe = bin_dir.join("codex.exe");
        std::fs::write(&shim, "#!/bin/sh\n").expect("shim");
        std::fs::write(&exe, "not-a-real-pe-but-good-enough-for-path-selection").expect("exe");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalize_windows_spawn_config(SpawnConfig {
            command: "codex".to_string(),
            args: vec!["--no-alt-screen".to_string()],
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        });

        assert_eq!(normalized.command, exe.display().to_string());
        assert_eq!(normalized.args, vec!["--no-alt-screen".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_resolves_shell_shim_js_entry_to_node() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        let node_modules = bin_dir
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin");
        std::fs::create_dir_all(&node_modules).expect("node modules");
        let shim = bin_dir.join("codex");
        let local_node = bin_dir.join("node.exe");
        let script = node_modules.join("codex.js");
        std::fs::write(&local_node, "not-a-real-pe").expect("node exe");
        std::fs::write(&script, "console.log('codex');\n").expect("script");
        std::fs::write(
            &shim,
            "#!/bin/sh\nif [ -x \"$basedir/node\" ]; then\n  exec \"$basedir/node\" \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nelse\n  exec node \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nfi\n",
        )
        .expect("shim");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalize_windows_spawn_config(SpawnConfig {
            command: "codex".to_string(),
            args: vec!["--no-alt-screen".to_string()],
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        });

        assert_eq!(normalized.command, local_node.display().to_string());
        assert_eq!(
            normalized.args,
            vec![script.display().to_string(), "--no-alt-screen".to_string(),]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_preserves_script_arg_for_node_exe_shims() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        let node_modules = bin_dir
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin");
        std::fs::create_dir_all(&node_modules).expect("node modules");
        let shim = bin_dir.join("codex");
        let local_node = bin_dir.join("node.exe");
        let script = node_modules.join("codex.js");
        std::fs::write(&local_node, "not-a-real-pe").expect("node exe");
        std::fs::write(&script, "console.log('codex');\n").expect("script");
        std::fs::write(
            &shim,
            "#!/bin/sh\nif [ -x \"$basedir/node.exe\" ]; then\n  exec \"$basedir/node.exe\" \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nelse\n  exec node \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nfi\n",
        )
        .expect("shim");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalize_windows_spawn_config(SpawnConfig {
            command: "codex".to_string(),
            args: vec!["--no-alt-screen".to_string()],
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        });

        assert_eq!(normalized.command, local_node.display().to_string());
        assert_eq!(
            normalized.args,
            vec![script.display().to_string(), "--no-alt-screen".to_string()]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_falls_back_to_node_when_shim_runtime_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        let node_modules = bin_dir
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin");
        std::fs::create_dir_all(&node_modules).expect("node modules");
        let shim = bin_dir.join("codex");
        let script = node_modules.join("codex.js");
        std::fs::write(&script, "console.log('codex');\n").expect("script");
        std::fs::write(
            &shim,
            "#!/bin/sh\nif [ -x \"$basedir/node.exe\" ]; then\n  exec \"$basedir/node.exe\" \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nelse\n  exec node \"$basedir/node_modules/@openai/codex/bin/codex.js\" \"$@\"\nfi\n",
        )
        .expect("shim");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalize_windows_spawn_config(SpawnConfig {
            command: "codex".to_string(),
            args: vec!["--no-alt-screen".to_string()],
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        });

        assert_eq!(normalized.command, "node");
        assert_eq!(
            normalized.args,
            vec![script.display().to_string(), "--no-alt-screen".to_string()]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_env_override_beats_remove_env() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), r"C:\custom\bin".to_string());

        let value = windows_env_value("PATH", &env, &[String::from("PATH")]);

        assert_eq!(value, Some(std::ffi::OsString::from(r"C:\custom\bin")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_spawn_succeeds_via_shell_shim_resolved_to_exe() {
        let _pty_guard = lock_pty_test();
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let shim = bin_dir.join("claude");
        let tool = bin_dir.join("tool.exe");
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot");
        let whoami = PathBuf::from(system_root)
            .join("System32")
            .join("whoami.exe");
        std::fs::copy(&whoami, &tool).expect("copy whoami");
        std::fs::write(&shim, "#!/bin/sh\nexec \"$basedir/tool.exe\" \"$@\"\n").expect("shim");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert(
            "PATHEXT".to_string(),
            ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC".to_string(),
        );

        let config = SpawnConfig {
            command: "claude".to_string(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        };

        let handle = PtyHandle::spawn(config).expect("spawn failed");
        assert!(handle.process_id().is_some(), "expected spawned process id");
    }

    #[cfg(windows)]
    #[test]
    fn windows_nodejs_distribution_npx_shim_does_not_collapse_to_node_exe() {
        // Regression for `node.exe: bad option: --yes` on Windows launches of
        // `npx --yes @pkg@version`. SPEC-1921 FR-081.
        //
        // Node.js's installer ships `npx` as a POSIX shell script whose
        // $basedir lookup only locates `node.exe` -- the actual CLI script
        // path is dereferenced via a separate `$CLI_BASEDIR` variable, so our
        // `$basedir/`-marker scan never pairs it with `npx-cli.js`. The parser
        // must refuse to substitute `node.exe` alone and fall back to the
        // `.cmd` sibling.
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("Program Files").join("nodejs");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");

        let npx_shim = bin_dir.join("npx");
        let npx_cmd = bin_dir.join("npx.cmd");
        let node_exe = bin_dir.join("node.exe");
        std::fs::write(&node_exe, "not-a-real-pe").expect("node exe placeholder");
        std::fs::write(
            &npx_shim,
            concat!(
                "#!/usr/bin/env bash\n",
                "basedir=`dirname \"$0\"`\n",
                "NODE_EXE=\"$basedir/node.exe\"\n",
                "if ! [ -x \"$NODE_EXE\" ]; then\n",
                "  NODE_EXE=\"$basedir/node\"\n",
                "fi\n",
                "CLI_BASEDIR=\"$(\"$NODE_EXE\" -p 'require(\"path\").dirname(process.execPath)' 2> /dev/null)\"\n",
                "NPX_CLI_JS=\"$CLI_BASEDIR/node_modules/npm/bin/npx-cli.js\"\n",
                "\"$NODE_EXE\" \"$NPX_CLI_JS\" \"$@\"\n",
            ),
        )
        .expect("npx shim");
        std::fs::write(
            &npx_cmd,
            concat!(
                "@ECHO OFF\n",
                "SET \"NODE_EXE=%~dp0\\node.exe\"\n",
                "\"%NODE_EXE%\" \"%~dp0\\node_modules\\npm\\bin\\npx-cli.js\" %*\n",
            ),
        )
        .expect("npx.cmd");

        let mut env = HashMap::new();
        env.insert("PATH".to_string(), bin_dir.display().to_string());
        env.insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());

        let normalized = normalize_windows_spawn_config(SpawnConfig {
            command: "npx".to_string(),
            args: vec![
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
            ],
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        });

        assert_ne!(
            normalized.command,
            node_exe.display().to_string(),
            "parser must not collapse a Node.js distribution shim to node.exe alone (FR-081): {:?} {:?}",
            normalized.command,
            normalized.args,
        );
        // The original agent args must survive unchanged somewhere in argv so
        // they still reach npx, not node.exe.
        assert!(
            normalized
                .args
                .iter()
                .any(|a| a == "@anthropic-ai/claude-code@latest"),
            "expected original package spec preserved in argv, got {:?}",
            normalized.args,
        );
        assert!(
            normalized.args.iter().any(|a| a == "--yes"),
            "expected --yes preserved in argv, got {:?}",
            normalized.args,
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_cmd_wrapper_omits_slash_s_flag() {
        // SPEC-1921 FR-082. `/s` makes CMD strip the quoting around the
        // executable path, which breaks `.cmd` invocations where the path
        // contains spaces (for example `C:\Program Files\nodejs\npx.cmd`).
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("Program Files").join("nodejs");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let cmd_path = bin_dir.join("npx.cmd");
        std::fs::write(&cmd_path, "@echo off\n").expect("cmd");

        let env: HashMap<String, String> = HashMap::new();
        let wrapped = windows_spawn_wrapper(&cmd_path, &env, &[]).expect("wrapper");

        assert!(
            !wrapped.1.iter().any(|a| a.eq_ignore_ascii_case("/s")),
            "cmd.exe wrapper must not include /s (FR-082), got argv {:?}",
            wrapped.1,
        );
        assert!(
            wrapped.1.iter().any(|a| a.eq_ignore_ascii_case("/d")),
            "wrapper should still include /d, got {:?}",
            wrapped.1,
        );
        assert!(
            wrapped.1.iter().any(|a| a.eq_ignore_ascii_case("/c")),
            "wrapper should still include /c, got {:?}",
            wrapped.1,
        );
    }
}
