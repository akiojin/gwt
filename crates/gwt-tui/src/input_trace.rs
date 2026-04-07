//! Opt-in input tracing for debugging terminal key routing.

use std::{
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
};

use crossterm::event::KeyEvent;
use serde::Serialize;

use crate::message::Message;

pub const INPUT_TRACE_PATH_ENV: &str = "GWT_INPUT_TRACE_PATH";

#[derive(Debug, Serialize)]
struct InputTraceRecord {
    timestamp: String,
    stage: &'static str,
    key_code: String,
    modifiers: String,
    kind: String,
    state: String,
    terminal_focused: Option<bool>,
    decision: Option<String>,
    session_id: Option<String>,
    bytes_hex: Option<String>,
}

impl InputTraceRecord {
    fn from_key(stage: &'static str, key: KeyEvent) -> Self {
        let (key_code, modifiers, kind, state) = key_fields(key);
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            stage,
            key_code,
            modifiers,
            kind,
            state,
            terminal_focused: None,
            decision: None,
            session_id: None,
            bytes_hex: None,
        }
    }

    fn from_keybind(key: KeyEvent, terminal_focused: bool, decision: Option<&Message>) -> Self {
        let mut record = Self::from_key("keybind", key);
        record.terminal_focused = Some(terminal_focused);
        record.decision = Some(
            decision
                .map(|message| format!("{message:?}"))
                .unwrap_or_else(|| "forward".to_string()),
        );
        record
    }

    fn from_pty_forward(key: KeyEvent, session_id: &str, bytes: &[u8]) -> Self {
        let mut record = Self::from_key("pty_forward", key);
        record.session_id = Some(session_id.to_string());
        record.bytes_hex = Some(bytes_to_hex(bytes));
        record
    }
}

pub fn trace_crossterm_key(key: KeyEvent) {
    let _ = append_if_configured(&InputTraceRecord::from_key("crossterm_key", key));
}

pub fn trace_keybind_decision(key: KeyEvent, terminal_focused: bool, decision: Option<&Message>) {
    let _ = append_if_configured(&InputTraceRecord::from_keybind(
        key,
        terminal_focused,
        decision,
    ));
}

pub fn trace_pty_forward(key: KeyEvent, session_id: &str, bytes: &[u8]) {
    let _ = append_if_configured(&InputTraceRecord::from_pty_forward(key, session_id, bytes));
}

fn append_if_configured(record: &InputTraceRecord) -> std::io::Result<()> {
    let Some(path) = configured_path() else {
        return Ok(());
    };
    append_record_with_path(&path, record)
}

fn configured_path() -> Option<PathBuf> {
    std::env::var_os(INPUT_TRACE_PATH_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn append_record_with_path(path: &Path, record: &InputTraceRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string(record)
        .map_err(|err| std::io::Error::other(format!("serialize input trace: {err}")))?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{json}")?;
    Ok(())
}

fn key_fields(key: KeyEvent) -> (String, String, String, String) {
    (
        format!("{:?}", key.code),
        format!("{:?}", key.modifiers),
        format!("{:?}", key.kind),
        format!("{:?}", key.state),
    )
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn append_record_with_path_writes_keybind_decision_jsonl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("input-trace.jsonl");
        let record =
            InputTraceRecord::from_keybind(key(KeyCode::Tab, KeyModifiers::NONE), true, None);

        append_record_with_path(&path, &record).expect("append input trace");

        let text = std::fs::read_to_string(&path).expect("read trace");
        assert!(text.contains("\"stage\":\"keybind\""));
        assert!(text.contains("\"terminal_focused\":true"));
        assert!(text.contains("\"decision\":\"forward\""));
    }

    #[test]
    fn append_record_with_path_writes_pty_forward_hex_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("input-trace.jsonl");
        let record = InputTraceRecord::from_pty_forward(
            key(KeyCode::Up, KeyModifiers::NONE),
            "shell-0",
            b"\x1b[A",
        );

        append_record_with_path(&path, &record).expect("append input trace");

        let text = std::fs::read_to_string(&path).expect("read trace");
        assert!(text.contains("\"stage\":\"pty_forward\""));
        assert!(text.contains("\"session_id\":\"shell-0\""));
        assert!(text.contains("\"bytes_hex\":\"1b5b41\""));
    }
}
