//! SPEC-1924 FR-040 / SC-013 regression test.
//!
//! `gwt.process.line` events must never reach the canonical JSONL log
//! file. They are observed by the in-process UI forwarder and the
//! `ProcessConsoleHub` only. `gwt.process.summary` events must reach
//! the file as usual.

use std::time::{Duration, Instant};

use gwt_core::logging::{current_log_file, init, LogLevel, LoggingConfig};

#[test]
fn process_line_events_are_excluded_from_canonical_log_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = LoggingConfig {
        log_dir: dir.path().to_path_buf(),
        default_level: LogLevel::Debug,
        config_file_level: None,
        retention_days: 0,
    };

    let handles = init(config).expect("init should succeed");

    // Emit at info level so the EnvFilter does not gate it. The
    // `gwt.process.line` exclusion happens at the fmt layer (target
    // prefix filter), not at the EnvFilter level.
    tracing::info!(
        target: "gwt.process.line",
        kind = "gh",
        stream = "stdout",
        "should NOT appear on disk"
    );
    tracing::info!(
        target: "gwt.process.summary",
        kind = "gh",
        spawn_id = 1u64,
        phase = "start",
        "summary should appear on disk"
    );
    tracing::info!(
        target: "gwt.process.summary",
        kind = "gh",
        spawn_id = 1u64,
        phase = "end",
        exit_code = 0i64,
        duration_ms = 42u64,
        "summary end should appear on disk"
    );

    drop(handles);

    let log_path = current_log_file(dir.path());
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut content = std::fs::read_to_string(&log_path).unwrap_or_default();
    while !content.contains("summary should appear on disk") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        content = std::fs::read_to_string(&log_path).unwrap_or_default();
    }

    assert!(
        content.contains("summary should appear on disk"),
        "expected summary start event in log file, got: {content}"
    );
    assert!(
        content.contains("summary end should appear on disk"),
        "expected summary end event in log file"
    );
    assert!(
        !content.contains("should NOT appear on disk"),
        "process line event leaked into canonical log: {content}"
    );
    assert!(
        !content.contains("\"target\":\"gwt.process.line\"")
            && !content.contains("\"target\": \"gwt.process.line\""),
        "no entry with target=gwt.process.line should be in canonical log: {content}"
    );
}
