//! Phase 8: integration tests for `gwt_core::index::watcher`.
//!
//! These tests exercise the debounce-and-batch behavior of the per-Worktree
//! filesystem watcher. They use the real `notify` crate against a tempdir.

use std::fs;
use std::time::Duration;

use gwt_core::index::watcher::{start_watcher, WatcherBatch, WatcherConfig};

fn write_file(dir: &std::path::Path, name: &str, contents: &str) {
    fs::write(dir.join(name), contents).unwrap();
}

#[tokio::test]
async fn burst_of_events_collapses_to_one_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = WatcherConfig {
        debounce: Duration::from_secs(2),
        batch_limit: 100,
    };
    let mut handle = start_watcher(tmp.path(), cfg).unwrap();

    for i in 0..50 {
        write_file(tmp.path(), &format!("f{i}.rs"), "// content\n");
    }

    let batch: WatcherBatch = tokio::time::timeout(Duration::from_secs(5), handle.recv_batch())
        .await
        .expect("watcher must emit batch within 5s")
        .expect("watcher channel must not close");

    let rs_paths: std::collections::HashSet<_> = batch
        .changed_paths
        .iter()
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rs"))
        .collect();
    assert_eq!(rs_paths.len(), 50);
    handle.shutdown().await;
}

#[tokio::test]
async fn batch_size_limit_splits_burst() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = WatcherConfig {
        debounce: Duration::from_secs(2),
        batch_limit: 100,
    };
    let mut handle = start_watcher(tmp.path(), cfg).unwrap();

    for i in 0..200 {
        write_file(tmp.path(), &format!("f{i}.rs"), "// c\n");
    }

    let mut total_rs: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    let mut saw_split = false;
    while total_rs.len() < 200 {
        let batch = tokio::time::timeout(Duration::from_secs(8), handle.recv_batch())
            .await
            .expect("expected next batch within 8s")
            .expect("channel open");
        assert!(
            batch.changed_paths.len() <= 100,
            "batch must respect 100 file limit (got {})",
            batch.changed_paths.len()
        );
        if batch.changed_paths.len() == 100 {
            saw_split = true;
        }
        for p in &batch.changed_paths {
            if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                total_rs.insert(p.clone());
            }
        }
    }
    assert_eq!(total_rs.len(), 200);
    assert!(saw_split, "expected at least one batch at the 100 limit");
    handle.shutdown().await;
}

#[tokio::test]
async fn gitignored_files_are_excluded() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join(".gitignore"), "ignored/\n").unwrap();
    fs::create_dir(tmp.path().join("ignored")).unwrap();
    fs::create_dir(tmp.path().join("kept")).unwrap();

    let cfg = WatcherConfig {
        debounce: Duration::from_secs(2),
        batch_limit: 100,
    };
    let mut handle = start_watcher(tmp.path(), cfg).unwrap();

    fs::write(tmp.path().join("ignored/should_skip.rs"), "// x\n").unwrap();
    fs::write(tmp.path().join("kept/should_keep.rs"), "// y\n").unwrap();

    let batch = tokio::time::timeout(Duration::from_secs(5), handle.recv_batch())
        .await
        .expect("watcher must emit batch")
        .expect("channel open");

    let paths: Vec<String> = batch
        .changed_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    assert!(
        !paths.iter().any(|p| p.contains("should_skip")),
        "gitignored path leaked into batch: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.contains("should_keep")),
        "kept path missing from batch: {paths:?}"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn watcher_shutdown_releases_resources() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = WatcherConfig::default();
    let handle = start_watcher(tmp.path(), cfg).unwrap();
    handle.shutdown().await;
    // No assertion necessary; the test passes if shutdown does not hang.
}
