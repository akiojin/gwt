//! SPEC-6 Phase 5 — end-to-end logging init test.
//!
//! Verifies that `gwt_core::logging::init` wires a non-blocking JSONL
//! writer that a `tracing::info!` call reaches within a short
//! deadline. Because `init` installs a **global** default subscriber
//! we must run this test in its own binary — hence the dedicated
//! integration test file.

use std::time::{Duration, Instant};

use gwt_core::logging::{current_log_file, init, LogLevel, LoggingConfig};

#[test]
fn init_writes_tracing_events_as_jsonl_to_gwt_log() {
    // SAFETY: `init` installs a global subscriber, so this test must
    // be the only one in this crate binary that calls it.
    let dir = tempfile::tempdir().expect("tempdir");
    let config = LoggingConfig {
        log_dir: dir.path().to_path_buf(),
        default_level: LogLevel::Debug,
        config_file_level: None,
        retention_days: 0, // disable housekeeping — test has a clean dir
    };

    let handles = init(config).expect("init should succeed");

    tracing::info!(
        target: "gwt_core::logging::test",
        session_id = "abc-123",
        "hello from test"
    );
    tracing::warn!(target: "gwt_core::logging::test", "warning sample");

    // Drop the handles BEFORE reading the log file. `WorkerGuard::drop`
    // sends a shutdown signal to the non-blocking writer thread and
    // joins it, which guarantees that every event emitted above has been
    // flushed to disk by the time this line returns. Polling the file
    // afterwards is redundant but kept as a short safety window in case
    // the filesystem itself takes a moment to make the write visible.
    drop(handles);

    let log_path = current_log_file(dir.path());
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut content = std::fs::read_to_string(&log_path).unwrap_or_default();
    while !content.contains("hello from test") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        content = std::fs::read_to_string(&log_path).unwrap_or_default();
    }

    assert!(
        content.contains("hello from test"),
        "expected hello event in log file, got: {content}"
    );
    assert!(
        content.contains("\"level\":\"INFO\"") || content.contains("\"level\": \"INFO\""),
        "expected level=INFO in log file, got: {content}"
    );
    assert!(
        content.contains("gwt_core::logging::test"),
        "expected target in log file"
    );
    assert!(
        content.contains("session_id") && content.contains("abc-123"),
        "expected structured field session_id=abc-123, got: {content}"
    );
    assert!(
        content.contains("warning sample"),
        "expected warn event in log file"
    );
}
