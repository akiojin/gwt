//! Extract file paths from the system clipboard.

use std::path::{Path, PathBuf};
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
        let Some(path) = parse_clipboard_path_line(line) else {
            return ClipboardPasteContent::Text(text.to_string());
        };
        paths.push(path);
    }

    ClipboardPasteContent::FilePaths(paths)
}

fn parse_clipboard_path_line(line: &str) -> Option<PathBuf> {
    if let Some(path) = parse_file_url(line) {
        return Some(path);
    }

    let path = PathBuf::from(line);
    path.is_absolute().then_some(path)
}

fn parse_file_url(line: &str) -> Option<PathBuf> {
    let raw_path = line.strip_prefix("file://")?;
    let raw_path = raw_path.strip_prefix("localhost").unwrap_or(raw_path);
    let decoded = percent_decode(raw_path)?;
    let path = PathBuf::from(decoded);
    path.is_absolute().then_some(path)
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hi = hex_value(bytes[index + 1])?;
                let lo = hex_value(bytes[index + 2])?;
                decoded.push((hi << 4) | lo);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Convert clipboard payload into bytes suitable for PTY input.
///
/// File paths take precedence over text. Empty text yields `None`.
pub fn clipboard_payload_to_bytes(paths: &[PathBuf], text: &str) -> Option<Vec<u8>> {
    if !paths.is_empty() {
        let joined = paths
            .iter()
            .map(|path| shell_quote_path(path.as_path()))
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

/// Quote a path so it can be pasted safely into a POSIX shell.
///
/// Each path is rendered on its own line and wrapped in single quotes. Single
/// quotes inside the path are escaped using the standard `'"'"'"'"'"'"'"'"'` sequence.
fn shell_quote_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut escaped = String::with_capacity(raw.len() + 2);
    escaped.push('\'');
    for ch in raw.chars() {
        if ch == '\'' {
            escaped.push_str("'\"'\"'");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_payload_to_bytes_quotes_paths_with_spaces_and_special_chars() {
        let paths = vec![PathBuf::from("/tmp/dir with spaces/it's $file&(1).txt")];
        let bytes = clipboard_payload_to_bytes(&paths, "").unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "'/tmp/dir with spaces/it'\"'\"'s $file&(1).txt'"
        );
    }

    #[test]
    fn clipboard_payload_to_bytes_joins_each_quoted_path_on_its_own_line() {
        let paths = vec![
            PathBuf::from("/tmp/one path"),
            PathBuf::from("/tmp/two path"),
        ];
        let bytes = clipboard_payload_to_bytes(&paths, "").unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "'/tmp/one path'\n'/tmp/two path'"
        );
    }

    #[test]
    fn clipboard_payload_to_bytes_keeps_text_when_no_paths_exist() {
        let bytes = clipboard_payload_to_bytes(&[], "plain text").unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "plain text");
    }

    #[test]
    fn parse_clipboard_paste_returns_file_paths_for_file_urls() {
        let parsed = parse_clipboard_paste("file:///Users/alice/Documents/report%20draft.txt\n");
        assert!(matches!(
            parsed,
            ClipboardPasteContent::FilePaths(paths)
                if paths == vec![PathBuf::from("/Users/alice/Documents/report draft.txt")]
        ));
    }

    #[test]
    fn parse_clipboard_paste_returns_multiple_file_paths_for_file_urls() {
        let parsed = parse_clipboard_paste(
            "file:///Users/alice/Documents/one.txt\nfile://localhost/Users/alice/Documents/two.txt\n",
        );
        assert!(matches!(
            parsed,
            ClipboardPasteContent::FilePaths(paths)
                if paths == vec![
                    PathBuf::from("/Users/alice/Documents/one.txt"),
                    PathBuf::from("/Users/alice/Documents/two.txt"),
                ]
        ));
    }
}
