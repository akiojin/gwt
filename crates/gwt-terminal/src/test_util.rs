//! Shared test utilities for gwt-terminal.

use std::{
    io::Read,
    sync::{mpsc, Mutex, MutexGuard, OnceLock},
    time::Duration,
};

static PTY_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn lock_pty_test() -> MutexGuard<'static, ()> {
    PTY_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug, Clone)]
pub struct TestCommand {
    pub command: String,
    pub args: Vec<String>,
}

#[cfg(windows)]
fn cmd_command(script: impl Into<String>) -> TestCommand {
    TestCommand {
        command: std::env::var("ComSpec")
            .or_else(|_| std::env::var("COMSPEC"))
            .unwrap_or_else(|_| "cmd.exe".to_string()),
        args: vec![
            "/d".to_string(),
            "/s".to_string(),
            "/c".to_string(),
            script.into(),
        ],
    }
}

pub fn echo_command(msg: &str) -> TestCommand {
    #[cfg(windows)]
    {
        cmd_command(format!("echo {msg}"))
    }

    #[cfg(not(windows))]
    {
        TestCommand {
            command: "/bin/echo".to_string(),
            args: vec![msg.to_string()],
        }
    }
}

pub fn sleep_command(secs: &str) -> TestCommand {
    #[cfg(windows)]
    {
        TestCommand {
            command: "powershell".to_string(),
            args: vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                format!("Start-Sleep -Seconds {secs}"),
            ],
        }
    }

    #[cfg(not(windows))]
    {
        TestCommand {
            command: "/bin/sleep".to_string(),
            args: vec![secs.to_string()],
        }
    }
}

pub fn stdin_echo_command() -> TestCommand {
    #[cfg(windows)]
    {
        TestCommand {
            command: "powershell".to_string(),
            args: vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "$line = [Console]::In.ReadLine(); Write-Output $line".to_string(),
            ],
        }
    }

    #[cfg(not(windows))]
    {
        TestCommand {
            command: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                "IFS= read -r line; printf '%s\n' \"$line\"".to_string(),
            ],
        }
    }
}

pub fn env_command() -> TestCommand {
    #[cfg(windows)]
    {
        cmd_command("set")
    }

    #[cfg(not(windows))]
    {
        TestCommand {
            command: "/usr/bin/env".to_string(),
            args: Vec::new(),
        }
    }
}

pub fn pwd_command() -> TestCommand {
    #[cfg(windows)]
    {
        cmd_command("cd")
    }

    #[cfg(not(windows))]
    {
        TestCommand {
            command: "/bin/pwd".to_string(),
            args: Vec::new(),
        }
    }
}

pub fn success_command() -> TestCommand {
    #[cfg(windows)]
    {
        cmd_command("exit /b 0")
    }

    #[cfg(not(windows))]
    {
        TestCommand {
            command: "/usr/bin/true".to_string(),
            args: Vec::new(),
        }
    }
}

pub fn answer_cursor_position_query(handle: &crate::pty::PtyHandle) {
    let _ = handle.write_input(b"\x1b[1;1R");
}

fn has_non_status_output(data: &[u8]) -> bool {
    String::from_utf8_lossy(data)
        .replace("\x1b[6n", "")
        .chars()
        .any(|ch| !ch.is_control())
}

/// Read from a PTY reader in a separate thread with timeout.
///
/// Returns accumulated output bytes, or an error message on failure/timeout.
pub fn read_with_timeout(
    mut reader: Box<dyn Read + Send>,
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
                if has_non_status_output(&last_output) {
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

pub fn read_until_contains(
    mut reader: Box<dyn Read + Send>,
    timeout: Duration,
    needle: &str,
) -> Result<Vec<u8>, String> {
    let needle = needle.to_string();
    let reader_needle = needle.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 4096];
        let mut output = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    output.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&output);
                    let _ = tx.send(Ok((output.clone(), text.contains(&reader_needle))));
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
            Ok(Ok((data, found))) => {
                last_output = data;
                if found {
                    return Ok(last_output);
                }
            }
            Ok(Err(e)) => return Err(e),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if last_output.is_empty() {
        Err(format!("Timed out before seeing {needle:?} with no output"))
    } else {
        Err(format!(
            "Timed out before seeing {needle:?}; last output: {}",
            String::from_utf8_lossy(&last_output)
        ))
    }
}
