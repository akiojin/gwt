//! Extract file paths from the system clipboard.

use std::path::PathBuf;
use std::process::Command;

/// Parsed clipboard paste payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardPasteContent {
    /// Clipboard content is a list of absolute file paths.
    FilePaths(Vec<PathBuf>),
    /// Clipboard content should be pasted as plain text.
    Text(String),
}

/// Clipboard-based file path extraction.
pub struct ClipboardFilePaste;

impl ClipboardFilePaste {
    /// Extract clipboard content as either file paths or text.
    pub fn extract_paste_content() -> Result<ClipboardPasteContent, ClipboardError> {
        let text = read_clipboard()?;
        Ok(parse_clipboard_paste(&text))
    }

    /// Extract file paths from the system clipboard.
    ///
    /// Returns absolute paths parsed from the clipboard content when the
    /// clipboard contains only absolute path lines. Otherwise returns an
    /// empty list so the caller can fall back to text pasting.
    pub fn extract_file_paths() -> Result<Vec<PathBuf>, ClipboardError> {
        match Self::extract_paste_content()? {
            ClipboardPasteContent::FilePaths(paths) => Ok(paths),
            ClipboardPasteContent::Text(_) => Ok(Vec::new()),
        }
    }
}

/// Parse clipboard text into file paths or plain text.
pub fn parse_clipboard_paste(text: &str) -> ClipboardPasteContent {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    if lines.is_empty() {
        return ClipboardPasteContent::Text(String::new());
    }

    let mut paths = Vec::with_capacity(lines.len());
    for line in lines {
        let path = PathBuf::from(line);
        if !path.is_absolute() {
            return ClipboardPasteContent::Text(text.to_string());
        }
        paths.push(path);
    }

    ClipboardPasteContent::FilePaths(paths)
}

/// Convert clipboard payload into bytes suitable for PTY input.
///
/// File paths take precedence over text. Empty text yields `None`.
pub fn clipboard_payload_to_bytes(paths: &[PathBuf], text: &str) -> Option<Vec<u8>> {
    if !paths.is_empty() {
        let joined = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        return Some(joined.into_bytes());
    }

    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(text.as_bytes().to_vec())
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
