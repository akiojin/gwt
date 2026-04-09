//! SPEC-6 Phase 5 — end-to-end Logs tab file watcher test.
//!
//! Verifies the full pipeline: append a JSONL line to the current
//! day's log file → `notify` debouncer fires → watcher parses →
//! `LogsWatcherPacket::AppendEntries` arrives on the receiver.

use std::io::Write;
use std::time::{Duration, Instant};

use gwt_core::logging::current_log_file;
use gwt_tui::logs_watcher::{self, LogsWatcherPacket};

fn recv_within(
    rx: &std::sync::mpsc::Receiver<LogsWatcherPacket>,
    deadline: Instant,
) -> Option<LogsWatcherPacket> {
    while Instant::now() < deadline {
        if let Ok(pkt) = rx.recv_timeout(Duration::from_millis(50)) {
            return Some(pkt);
        }
    }
    None
}

#[test]
fn watcher_initial_read_then_append_delivers_both_packets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log_path = current_log_file(dir.path());

    // Pre-populate the log file with two entries so the initial read
    // has something to SetEntries with.
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap();
        writeln!(
            f,
            r#"{{"timestamp":"2026-04-08T10:00:00+09:00","level":"INFO","target":"gwt_tui::main","fields":{{"message":"first"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"timestamp":"2026-04-08T10:00:01+09:00","level":"WARN","target":"gwt_tui::main","fields":{{"message":"second"}}}}"#
        )
        .unwrap();
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let _handle = logs_watcher::spawn(dir.path().to_path_buf(), tx);

    // 1) First packet should be the initial full read.
    let deadline = Instant::now() + Duration::from_secs(5);
    let initial = recv_within(&rx, deadline).expect("initial SetEntries packet");
    match initial {
        LogsWatcherPacket::SetEntries(entries) => {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].message, "first");
            assert_eq!(entries[1].message, "second");
        }
        other => panic!("expected SetEntries, got {other:?}"),
    }

    // 2) Append a third line. Expect an AppendEntries packet.
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap();
        writeln!(
            f,
            r#"{{"timestamp":"2026-04-08T10:00:02+09:00","level":"ERROR","target":"gwt_tui::main","fields":{{"message":"third","detail":"boom"}}}}"#
        )
        .unwrap();
        f.sync_all().unwrap();
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    let follow_up = recv_within(&rx, deadline).expect("AppendEntries packet after write");
    match follow_up {
        LogsWatcherPacket::AppendEntries(entries) => {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].message, "third");
            assert_eq!(entries[0].detail.as_deref(), Some("boom"));
        }
        other => panic!("expected AppendEntries, got {other:?}"),
    }
}
