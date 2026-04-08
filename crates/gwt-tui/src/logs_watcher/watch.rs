//! File watcher for `gwt.log.YYYY-MM-DD`.
//!
//! Spawns a background thread that:
//!
//! 1. Performs an initial full read of the current day's log file and
//!    dispatches a `LogsMessage::SetEntries` to the UI.
//! 2. Installs a debounced `notify` watcher on the log directory.
//! 3. On each debounced change, reads any new bytes appended since
//!    the previous offset, parses them as JSONL, and dispatches a
//!    `LogsMessage::AppendEntries`.
//! 4. Handles day rollover by reopening the new `gwt.log.YYYY-MM-DD`
//!    file when it appears.

use std::io::{BufRead, BufReader, Seek, SeekFrom};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::Duration;

use gwt_core::logging::{current_log_file, LogEvent};
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;

use super::parser::parse_line;

/// A packet of events produced by the watcher thread. The main loop
/// drains these each tick and feeds them to `LogsState`.
#[derive(Debug, Clone)]
pub enum LogsWatcherPacket {
    /// Full replacement — used for the initial read and on refresh.
    SetEntries(Vec<LogEvent>),
    /// Incremental append.
    AppendEntries(Vec<LogEvent>),
}

/// Public handle returned by `spawn`.
pub struct LogsWatcherHandle {
    _thread: thread::JoinHandle<()>,
}

/// Spawn the background watcher thread. Errors during initial setup
/// are reported via `tracing::warn!` and the thread exits cleanly;
/// the Logs tab will simply remain empty rather than bringing down
/// the TUI.
pub fn spawn(log_dir: PathBuf, tx: Sender<LogsWatcherPacket>) -> LogsWatcherHandle {
    let handle = thread::Builder::new()
        .name("gwt-logs-watcher".into())
        .spawn(move || run(log_dir, tx))
        .expect("spawn logs watcher thread");
    LogsWatcherHandle { _thread: handle }
}

fn run(log_dir: PathBuf, tx: Sender<LogsWatcherPacket>) {
    let mut current = WatcherState::new(log_dir.clone());

    // Initial read: parse the entire current day's file (if it exists)
    // and dispatch SetEntries.
    let entries = current.read_all_from_start();
    if tx.send(LogsWatcherPacket::SetEntries(entries)).is_err() {
        return;
    }

    // notify-debouncer-mini uses a separate sender/receiver pair.
    let (evt_tx, evt_rx) = channel();
    let mut debouncer = match new_debouncer(Duration::from_millis(150), evt_tx) {
        Ok(d) => d,
        Err(err) => {
            tracing::warn!(
                target: "gwt_tui::logs_watcher",
                error = %err,
                "failed to initialize file watcher; Logs tab will not stream"
            );
            return;
        }
    };

    if let Err(err) = debouncer
        .watcher()
        .watch(&log_dir, RecursiveMode::NonRecursive)
    {
        tracing::warn!(
            target: "gwt_tui::logs_watcher",
            dir = %log_dir.display(),
            error = %err,
            "failed to watch log directory; Logs tab will not stream"
        );
        return;
    }

    // Debouncer events keep coming until the main process exits.
    // `recv` returns `Err` when the sender is dropped, which happens
    // on thread shutdown.
    while let Ok(events) = evt_rx.recv() {
        // `events` is a `Result<Vec<DebouncedEvent>, Vec<notify::Error>>`
        // — treat errors as "retry by reopening the file".
        if events.is_err() {
            continue;
        }

        // The watcher reports paths; we only care that *something*
        // happened in the log dir. Re-check the current log path
        // (to catch date rollover) and drain new bytes.
        if current.maybe_rotate() {
            let replacement = current.read_all_from_start();
            if tx.send(LogsWatcherPacket::SetEntries(replacement)).is_err() {
                return;
            }
            continue;
        }
        let appended = current.read_tail();
        if !appended.is_empty() && tx.send(LogsWatcherPacket::AppendEntries(appended)).is_err() {
            return;
        }
    }
}

/// Per-file bookkeeping: which dated file we are currently tailing
/// and how many bytes we have already shipped.
struct WatcherState {
    log_dir: PathBuf,
    current_file: PathBuf,
    offset: u64,
}

impl WatcherState {
    fn new(log_dir: PathBuf) -> Self {
        let current_file = current_log_file(&log_dir);
        Self {
            log_dir,
            current_file,
            offset: 0,
        }
    }

    /// Check whether the "current day" file has changed (rollover at
    /// local midnight). Returns `true` if the pointer was reset.
    fn maybe_rotate(&mut self) -> bool {
        let today = current_log_file(&self.log_dir);
        if today != self.current_file {
            self.current_file = today;
            self.offset = 0;
            return true;
        }
        false
    }

    /// Read the full file from byte 0, resetting the offset counter.
    /// Used for the initial read and after rollover.
    fn read_all_from_start(&mut self) -> Vec<LogEvent> {
        self.offset = 0;
        self.read_tail()
    }

    /// Read from `self.offset` to EOF and return parsed events.
    fn read_tail(&mut self) -> Vec<LogEvent> {
        let file = match std::fs::File::open(&self.current_file) {
            Ok(f) => f,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(err) => {
                tracing::warn!(
                    target: "gwt_tui::logs_watcher",
                    path = %self.current_file.display(),
                    error = %err,
                    "failed to open log file"
                );
                return Vec::new();
            }
        };

        let metadata = match file.metadata() {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };
        let len = metadata.len();

        // File shrank ⇒ assume truncation/rotation and re-read from
        // the start. (tracing-appender does not truncate, but this
        // is a safety net.)
        if len < self.offset {
            self.offset = 0;
        }

        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }

        let mut events = Vec::new();
        let mut buffer = String::new();
        let mut total_read: u64 = 0;
        loop {
            buffer.clear();
            let n = match reader.read_line(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            total_read += n as u64;
            // Partial line (no trailing newline) means the writer is
            // mid-write. Don't advance the offset past it — we'll pick
            // it up on the next tick when the newline has arrived.
            if !buffer.ends_with('\n') {
                total_read -= n as u64;
                break;
            }
            events.push(parse_line(&buffer));
        }
        self.offset += total_read;
        events
    }

    /// Expose the current file for tests / diagnostics.
    #[cfg(test)]
    fn current_file(&self) -> &Path {
        &self.current_file
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_line(path: &Path, line: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f, "{line}").unwrap();
        f.sync_all().unwrap();
    }

    #[test]
    fn read_tail_returns_empty_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = WatcherState::new(dir.path().to_path_buf());
        assert!(state.read_tail().is_empty());
    }

    #[test]
    fn read_tail_returns_new_lines_and_advances_offset() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = WatcherState::new(dir.path().to_path_buf());
        let path = state.current_file().to_path_buf();
        write_line(
            &path,
            r#"{"timestamp":"2026-04-08T10:00:00+09:00","level":"INFO","target":"t","fields":{"message":"one"}}"#,
        );
        write_line(
            &path,
            r#"{"timestamp":"2026-04-08T10:00:01+09:00","level":"WARN","target":"t","fields":{"message":"two"}}"#,
        );
        let first = state.read_tail();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].message, "one");
        assert_eq!(first[1].message, "two");
        let second = state.read_tail();
        assert!(second.is_empty());
        write_line(
            &path,
            r#"{"timestamp":"2026-04-08T10:00:02+09:00","level":"ERROR","target":"t","fields":{"message":"three"}}"#,
        );
        let third = state.read_tail();
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].message, "three");
    }

    #[test]
    fn read_all_from_start_resets_offset() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = WatcherState::new(dir.path().to_path_buf());
        let path = state.current_file().to_path_buf();
        for i in 0..5 {
            write_line(
                &path,
                &format!(
                    r#"{{"timestamp":"2026-04-08T10:00:0{i}+09:00","level":"INFO","target":"t","fields":{{"message":"m{i}"}}}}"#
                ),
            );
        }
        let first = state.read_tail();
        assert_eq!(first.len(), 5);
        assert!(state.read_tail().is_empty());
        let again = state.read_all_from_start();
        assert_eq!(again.len(), 5);
    }
}
