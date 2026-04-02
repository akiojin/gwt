//! Extract file paths from the system clipboard.

use std::path::PathBuf;
use std::process::Command;

/// Clipboard-based file path extraction.
pub struct ClipboardFilePaste;

impl ClipboardFilePaste {
    /// Extract file paths from the system clipboard.
    ///
    /// Returns absolute paths parsed from the clipboard content (newline-separated).
    /// Uses platform-specific tools: `pbpaste` on macOS, `xclip`/`wl-paste` on Linux.
    pub fn extract_file_paths() -> Result<Vec<PathBuf>, ClipboardError> {
        let text = read_clipboard()?;
        let paths: Vec<PathBuf> = text
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect();
        Ok(paths)
    }
}

// ── Shared clipboard helpers (crate-internal) ──

/// Read text from the system clipboard using platform-specific tools.
pub(crate) fn read_clipboard() -> Result<String, ClipboardError> {
    if cfg!(target_os = "macos") {
        run_command("pbpaste", &[])
    } else if cfg!(target_os = "linux") {
        // Try wl-paste first (Wayland), fall back to xclip (X11)
        run_command("wl-paste", &[])
            .or_else(|_| run_command("xclip", &["-selection", "clipboard", "-o"]))
    } else {
        Err(ClipboardError::UnsupportedPlatform)
    }
}

/// Write text to the system clipboard using platform-specific tools.
pub(crate) fn write_clipboard(text: &str) -> Result<(), ClipboardError> {
    if cfg!(target_os = "macos") {
        pipe_to_command("pbcopy", &[], text)
    } else if cfg!(target_os = "linux") {
        pipe_to_command("wl-copy", &[], text)
            .or_else(|_| pipe_to_command("xclip", &["-selection", "clipboard"], text))
    } else {
        Err(ClipboardError::UnsupportedPlatform)
    }
}

/// Run a command and capture its stdout as a String.
pub(crate) fn run_command(cmd: &str, args: &[&str]) -> Result<String, ClipboardError> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| ClipboardError::CommandFailed(format!("{cmd}: {e}")))?;

    if !output.status.success() {
        return Err(ClipboardError::CommandFailed(format!(
            "{cmd} exited with {}",
            output.status
        )));
    }

    String::from_utf8(output.stdout).map_err(|e| ClipboardError::InvalidUtf8(e.to_string()))
}

/// Pipe text into a command's stdin.
fn pipe_to_command(cmd: &str, args: &[&str], text: &str) -> Result<(), ClipboardError> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| ClipboardError::CommandFailed(format!("{cmd}: {e}")))?;

    if let Some(ref mut stdin) = child.stdin {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| ClipboardError::CommandFailed(format!("{cmd} stdin: {e}")))?;
    }

    let status = child
        .wait()
        .map_err(|e| ClipboardError::CommandFailed(format!("{cmd} wait: {e}")))?;

    if !status.success() {
        return Err(ClipboardError::CommandFailed(format!(
            "{cmd} exited with {status}"
        )));
    }

    Ok(())
}

/// Errors produced by clipboard operations.
#[derive(Debug, thiserror::Error)]
pub enum ClipboardError {
    #[error("Clipboard command failed: {0}")]
    CommandFailed(String),

    #[error("Clipboard content is not valid UTF-8: {0}")]
    InvalidUtf8(String),

    #[error("Unsupported platform for clipboard access")]
    UnsupportedPlatform,
}
