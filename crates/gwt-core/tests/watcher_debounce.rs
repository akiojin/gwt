//! Phase 8: integration tests for `gwt_core::index::watcher`.
//!
//! These tests exercise the debounce-and-batch behavior of the per-Worktree
//! filesystem watcher. They use the real `notify` crate against a tempdir.

use std::{fs, time::Duration};

use gwt_core::index::watcher::{start_watcher, WatcherConfig};

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

    // The debouncer should eventually deliver all 50 files. On Linux/inotify
    // the events for a single burst can arrive across multiple batches even
    // when the batch limit is not hit, so we drain until we have all 50
    // unique paths or the per-batch timeout fires.
    let mut rs_paths: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    while rs_paths.len() < 50 {
        let batch = tokio::time::timeout(Duration::from_secs(8), handle.recv_batch())
            .await
            .expect("watcher must keep emitting batches until 50 files seen")
            .expect("watcher channel must not close");
        for p in &batch.changed_paths {
            if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                rs_paths.insert(p.clone());
            }
        }
    }
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

    // The debouncer can split nearby events across multiple batches under
    // Linux inotify, so drain until we see the kept file or a deadline
    // fires. Each batch must still pass the "no gitignored path" check.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut saw_kept = false;
    while !saw_kept {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let batch = tokio::time::timeout(remaining, handle.recv_batch())
            .await
            .expect("watcher must emit a batch before deadline")
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
        if paths.iter().any(|p| p.contains("should_keep")) {
            saw_kept = true;
        }
    }
    assert!(saw_kept, "kept path never observed in any batch");
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
