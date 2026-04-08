//! Structured logging foundation (SPEC-6 Phase 5).
//!
//! Provides the single initialization entry point `init()` that wires a
//! `tracing-subscriber` Registry with:
//!
//! 1. A reloadable `EnvFilter` (level control via `reload::Handle`)
//! 2. A JSONL formatting layer writing to `~/.gwt/logs/gwt.log` via a
//!    non-blocking, daily-rolling appender (`tracing_appender`)
//! 3. A UI forwarder layer that sends `LogEvent`s to an
//!    `UnboundedSender<LogEvent>` so that TUI surfaces (toasts, error
//!    modal) can react to `Info`/`Warn`/`Error` events without
//!    parsing the log file.
//!
//! The file is the single source of truth for the Logs tab. The UI
//! forwarder channel is only used to drive ephemeral surfaces that
//! must react within one UI tick.
//!
//! See `specs/SPEC-6/spec.md` Phase 5 and `specs/SPEC-6/plan.md` Phase 5
//! for the architectural background.

pub mod config;
pub mod event;
pub mod fmt_layer;
pub mod housekeep;
pub mod init;
pub mod ui_forwarder;
pub mod writer;

pub use config::{LogLevel, LoggingConfig};
pub use event::LogEvent;
pub use housekeep::{housekeep, HousekeepReport};
pub use init::{init, LoggingHandles, ReloadHandle};
pub use writer::{current_log_file, log_file_for_date, LOG_FILE_BASENAME};
