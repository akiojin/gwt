//! Process console — ephemeral hub for external process stdout / stderr.
//!
//! See SPEC-1924 Update 2026-05-20 (Process Console Domain) and SPEC-2019
//! Update 2026-05-20 (Process Console Facet) for the motivating discussion.
//!
//! ## Architecture
//!
//! Three pieces collaborate:
//!
//! 1. [`ProcessKind`] enum — closed set of process categories that gwt
//!    spawns: gh / git / docker / agent bootstrap / Python index runner.
//! 2. [`ProcessConsoleHub`] — ring-buffer + broadcast surface that the
//!    Logs window subscribes to via WebSocket. Owned by `LoggingHandles`.
//! 3. [`spawn_logged`] — single entry point that callers use to launch
//!    external processes. Emits `gwt.process.summary` tracing events to
//!    the canonical log file and forwards stdout / stderr lines (after
//!    redaction) to the hub.
//!
//! Line-level events never reach the canonical log file. They live only
//! in the hub's ring buffer (capacity 5000 lines / kind by default) and
//! the broadcast channel. The summary events (start / end / exit_code /
//! duration / line counts) are persisted to the canonical file via the
//! standard tracing pipeline.

pub mod hub;
pub mod kind;
pub mod line;
pub mod redact;
pub mod spawn;

pub use hub::{global, set_global, ProcessConsoleHub, DEFAULT_RING_CAPACITY};
pub use kind::{ParseProcessKindError, ProcessKind};
pub use line::{ProcessLine, ProcessStream};
pub use redact::{redact_line, REDACTED};
pub use spawn::{spawn_logged, spawn_logged_blocking, SpawnOptions, SpawnOutput};
