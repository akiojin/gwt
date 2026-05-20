//! `ProcessLine` — a single stdout / stderr line emitted by an external process.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::kind::ProcessKind;

/// Which stdio stream produced the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessStream {
    #[serde(rename = "stdout")]
    Stdout,
    #[serde(rename = "stderr")]
    Stderr,
}

impl ProcessStream {
    pub fn as_str(self) -> &'static str {
        match self {
            ProcessStream::Stdout => "stdout",
            ProcessStream::Stderr => "stderr",
        }
    }
}

/// One redacted stdout / stderr line plus enough metadata to filter and
/// render it in the Logs window's Process facet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLine {
    /// Logical category of the spawning process.
    pub kind: ProcessKind,
    /// Per-spawn correlation id. Lets the UI group lines that came from
    /// the same `spawn_logged` call across kinds.
    pub spawn_id: u64,
    /// Which stdio stream produced the line.
    pub stream: ProcessStream,
    /// The redacted line text. Trailing line separators are stripped
    /// before redaction. Carriage-return progress lines (`docker pull`,
    /// `git clone`) are split into one line per CR by the spawn loop.
    pub message: String,
    /// Local time at which `spawn_logged` observed the line.
    pub timestamp: DateTime<Utc>,
}

impl ProcessLine {
    pub fn new(
        kind: ProcessKind,
        spawn_id: u64,
        stream: ProcessStream,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            spawn_id,
            stream,
            message: message.into(),
            timestamp: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_stream_roundtrip_via_serde() {
        assert_eq!(
            serde_json::to_string(&ProcessStream::Stdout).unwrap(),
            "\"stdout\""
        );
        let parsed: ProcessStream = serde_json::from_str("\"stderr\"").unwrap();
        assert_eq!(parsed, ProcessStream::Stderr);
    }

    #[test]
    fn process_line_carries_kind_and_text() {
        let line = ProcessLine::new(
            ProcessKind::Gh,
            42,
            ProcessStream::Stdout,
            "PR #123 created",
        );
        assert_eq!(line.kind, ProcessKind::Gh);
        assert_eq!(line.spawn_id, 42);
        assert_eq!(line.stream, ProcessStream::Stdout);
        assert_eq!(line.message, "PR #123 created");
    }

    #[test]
    fn process_line_roundtrip_through_json() {
        let line = ProcessLine::new(
            ProcessKind::Docker,
            7,
            ProcessStream::Stderr,
            "Pulling fs layer",
        );
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.contains("\"docker\""));
        assert!(json.contains("\"stderr\""));
        let back: ProcessLine = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, ProcessKind::Docker);
        assert_eq!(back.stream, ProcessStream::Stderr);
        assert_eq!(back.message, "Pulling fs layer");
    }
}
