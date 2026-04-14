//! Opt-in input tracing for debugging terminal key routing.

use std::{
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
    time::Duration,
};

use crossterm::event::{Event, KeyEvent};
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

#[derive(Debug, Serialize)]
struct ProbeTraceRecord {
    timestamp: String,
    event_type: &'static str,
    event_debug: String,
    key_code: Option<String>,
    modifiers: Option<String>,
    kind: Option<String>,
    state: Option<String>,
    paste_text: Option<String>,
    columns: Option<u16>,
    rows: Option<u16>,
}

#[derive(Debug, Serialize)]
struct DispatchTraceRecord {
    timestamp: String,
    stage: &'static str,
    message: String,
    elapsed_us: u128,
    detail: Option<String>,
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

impl ProbeTraceRecord {
    fn from_event(event: &Event) -> Self {
        let mut record = Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            event_type: probe_event_type(event),
            event_debug: format!("{event:?}"),
            key_code: None,
            modifiers: None,
            kind: None,
            state: None,
            paste_text: None,
            columns: None,
            rows: None,
        };

        match event {
            Event::Key(key) => {
                let (key_code, modifiers, kind, state) = key_fields(*key);
                record.key_code = Some(key_code);
                record.modifiers = Some(modifiers);
                record.kind = Some(kind);
                record.state = Some(state);
            }
            Event::Paste(text) => {
                record.paste_text = Some(text.clone());
            }
            Event::Resize(columns, rows) => {
                record.columns = Some(*columns);
                record.rows = Some(*rows);
            }
            Event::FocusGained | Event::FocusLost | Event::Mouse(_) => {}
        }

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

pub fn trace_dispatch_timing(
    stage: &'static str,
    message: &str,
    elapsed: Duration,
    detail: Option<&str>,
) {
    let _ = append_dispatch_if_configured(&DispatchTraceRecord {
        timestamp: chrono::Utc::now().to_rfc3339(),
        stage,
        message: message.to_string(),
        elapsed_us: elapsed.as_micros(),
        detail: detail.map(str::to_string),
    });
}

pub fn append_probe_event_with_path(path: &Path, event: &Event) -> std::io::Result<()> {
    append_probe_record_with_path(path, &ProbeTraceRecord::from_event(event))
}

fn append_if_configured(record: &InputTraceRecord) -> std::io::Result<()> {
    let Some(path) = configured_path() else {
        return Ok(());
    };
    append_record_with_path(&path, record)
}

fn append_dispatch_if_configured(record: &DispatchTraceRecord) -> std::io::Result<()> {
    let Some(path) = configured_path() else {
        return Ok(());
    };
    append_dispatch_record_with_path(&path, record)
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

fn append_probe_record_with_path(path: &Path, record: &ProbeTraceRecord) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string(record)
        .map_err(|err| std::io::Error::other(format!("serialize probe trace: {err}")))?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{json}")?;
    Ok(())
}

fn append_dispatch_record_with_path(
    path: &Path,
    record: &DispatchTraceRecord,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string(record)
        .map_err(|err| std::io::Error::other(format!("serialize dispatch trace: {err}")))?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{json}")?;
    Ok(())
}

fn key_fields(key: KeyEvent) -> (String, String, String, String) {
    (
        format!("{:?}", key.code),
        key_modifiers_string(key.modifiers),
        format!("{:?}", key.kind),
        format!("{:?}", key.state),
    )
}

fn key_modifiers_string(modifiers: crossterm::event::KeyModifiers) -> String {
    if modifiers.is_empty() {
        return "NONE".to_string();
    }

    let mut labels = Vec::new();
    if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
        labels.push("SHIFT");
    }
    if modifiers.contains(crossterm::event::KeyModifiers::CONTROL) {
        labels.push("CONTROL");
    }
    if modifiers.contains(crossterm::event::KeyModifiers::ALT) {
        labels.push("ALT");
    }
    if modifiers.contains(crossterm::event::KeyModifiers::SUPER) {
        labels.push("SUPER");
    }
    if modifiers.contains(crossterm::event::KeyModifiers::HYPER) {
        labels.push("HYPER");
    }
    if modifiers.contains(crossterm::event::KeyModifiers::META) {
        labels.push("META");
    }
    labels.join("|")
}

fn probe_event_type(event: &Event) -> &'static str {
    match event {
        Event::FocusGained => "focus_gained",
        Event::FocusLost => "focus_lost",
        Event::Key(_) => "key",
        Event::Mouse(_) => "mouse",
        Event::Paste(_) => "paste",
        Event::Resize(_, _) => "resize",
    }
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

    #[test]
    fn append_dispatch_record_with_path_writes_dispatch_timing_jsonl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("input-trace.jsonl");
        let record = DispatchTraceRecord {
            timestamp: chrono::Utc::now().to_rfc3339(),
            stage: "wizard_dispatch",
            message: "MoveDown".to_string(),
            elapsed_us: 123,
            detail: Some("docker_sync=skipped".to_string()),
        };

        append_dispatch_record_with_path(&path, &record).expect("append dispatch trace");

        let text = std::fs::read_to_string(&path).expect("read trace");
        assert!(text.contains("\"stage\":\"wizard_dispatch\""));
        assert!(text.contains("\"message\":\"MoveDown\""));
        assert!(text.contains("\"elapsed_us\":123"));
        assert!(text.contains("\"detail\":\"docker_sync=skipped\""));
    }

    #[test]
    fn append_probe_event_with_path_writes_key_event_jsonl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("probe-trace.jsonl");

        append_probe_event_with_path(&path, &Event::Key(key(KeyCode::Tab, KeyModifiers::SHIFT)))
            .expect("append probe trace");

        let text = std::fs::read_to_string(&path).expect("read trace");
        assert!(text.contains("\"event_type\":\"key\""));
        assert!(text.contains("\"key_code\":\"Tab\""));
        assert!(text.contains("\"modifiers\":\"SHIFT\""));
        assert!(text.contains("\"kind\":\"Press\""));
    }

    #[test]
    fn append_probe_event_with_path_writes_paste_event_jsonl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("probe-trace.jsonl");

        append_probe_event_with_path(&path, &Event::Paste("nihongo".into()))
            .expect("append probe trace");

        let text = std::fs::read_to_string(&path).expect("read trace");
        assert!(text.contains("\"event_type\":\"paste\""));
        assert!(text.contains("\"event_debug\":\"Paste(\\\"nihongo\\\")\""));
        assert!(text.contains("\"paste_text\":\"nihongo\""));
    }
}
