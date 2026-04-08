//! Non-blocking, daily-rolling JSONL writer backed by `tracing_appender`.
//!
//! **File naming:** `tracing_appender::rolling::daily` does not maintain
//! a bare active file. Every event is written to
//! `{log_dir}/gwt.log.YYYY-MM-DD` where the date is the current local
//! day. The Logs tab and housekeeping code must therefore reference the
//! dated filename directly. Use `current_log_file()` to compute it.

use std::path::{Path, PathBuf};

use chrono::Local;
use tracing_appender::{
    non_blocking::{NonBlocking, WorkerGuard},
    rolling,
};

/// Basename prefix used for the rolling log files. The full name is
/// `gwt.log.YYYY-MM-DD` (no bare `gwt.log` file exists at any point).
pub const LOG_FILE_BASENAME: &str = "gwt.log";

/// Return the path of today's active log file
/// (`{log_dir}/gwt.log.YYYY-MM-DD`, local date).
pub fn current_log_file(log_dir: &Path) -> PathBuf {
    let today = Local::now().date_naive();
    log_dir.join(format!("{LOG_FILE_BASENAME}.{today}"))
}

/// Return the path for the log file of a specific local date. Used by
/// the file watcher when a date rollover is observed.
pub fn log_file_for_date(log_dir: &Path, date: chrono::NaiveDate) -> PathBuf {
    log_dir.join(format!("{LOG_FILE_BASENAME}.{date}"))
}

/// Create a daily-rolling, non-blocking writer targeting `log_dir/gwt.log`.
///
/// The returned `WorkerGuard` must be kept alive (for example in a
/// `LoggingHandles` held by `main`) until the process exits, otherwise
/// the background writer thread shuts down and events are dropped.
pub fn build(log_dir: &Path) -> std::io::Result<(NonBlocking, WorkerGuard)> {
    std::fs::create_dir_all(log_dir)?;

    // `rolling::daily` uses the system timezone for boundary detection in
    // recent `tracing-appender` versions; older versions use UTC. We accept
    // the library's behaviour here and document the contract via
    // `specs/SPEC-6/plan.md` Phase 5 — the acceptance test in
    // `crates/gwt-core/tests/logging_init.rs` verifies end-to-end
    // behaviour regardless of exact timezone.
    let file_appender = rolling::daily(log_dir, LOG_FILE_BASENAME);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    Ok((non_blocking, guard))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_creates_log_dir_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b");
        assert!(!nested.exists());
        let (_writer, _guard) = build(&nested).expect("build writer");
        assert!(nested.is_dir());
    }
}
