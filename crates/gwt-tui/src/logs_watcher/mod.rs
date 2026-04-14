//! Logs tab file watcher (SPEC-6 Phase 5).
//!
//! Tails the active project's
//! `~/.gwt/logs/<repo-hash>/gwt.log.YYYY-MM-DD`, parses each appended
//! JSONL line into a `LogEvent`, and dispatches batches to the main TUI
//! loop over a `std::sync::mpsc::Sender<LogsWatcherPacket>`.
//!
//! The file is the single source of truth for the Logs tab; the
//! in-memory ring buffer retired with Phase 5 is gone.

pub mod parser;
pub mod watch;

pub use parser::parse_line;
pub use watch::{spawn, LogsWatcherHandle, LogsWatcherPacket};
