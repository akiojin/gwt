//! Deterministic integration tests for the real debounce/filter/batch pipeline.
//! The fixture source supplies raw notifications without relying on OS delivery.
//! Native delivery has its own explicitly invoked probe in watcher_native.rs.

use std::{collections::HashSet, fs, path::PathBuf, time::Duration};

use gwt_core::index::watcher::{fixture::start_fixture_watcher, start_watcher, WatcherConfig};

fn config() -> WatcherConfig {
    WatcherConfig {
        debounce: Duration::from_secs(2),
        batch_limit: 100,
    }
}

#[tokio::test]
async fn burst_of_events_collapses_to_one_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap();
    let expected: HashSet<PathBuf> = (0..50)
        .map(|i| {
            let path = root.join(format!("f{i}.rs"));
            fs::write(&path, "// content\n").unwrap();
            path
        })
        .collect();
    let mut handle = start_fixture_watcher(&root, config()).unwrap();
    let batch = handle
        .recv_batch()
        .await
        .expect("watcher channel must remain open");
    assert_eq!(batch.changed_paths.len(), 50);
    assert_eq!(
        batch.changed_paths.into_iter().collect::<HashSet<_>>(),
        expected
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn gitignored_files_are_excluded() {
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap();
    fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    fs::create_dir(root.join("ignored")).unwrap();
    fs::create_dir(root.join("kept")).unwrap();
    fs::write(root.join("ignored/should_skip.rs"), "// x\n").unwrap();
    let kept = root.join("kept/should_keep.rs");
    fs::write(&kept, "// y\n").unwrap();

    let mut handle = start_fixture_watcher(&root, config()).unwrap();
    let batch = handle
        .recv_batch()
        .await
        .expect("watcher channel must remain open");
    assert_eq!(
        batch.changed_paths.into_iter().collect::<HashSet<_>>(),
        HashSet::from([kept, root.join(".gitignore")])
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn nested_gitignored_files_are_excluded() {
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap();
    let app = root.join("packages/app");
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join(".gitignore"), "*.generated\n").unwrap();
    fs::write(app.join("view.generated"), "ignored\n").unwrap();
    let kept = app.join("view.rs");
    fs::write(&kept, "// kept\n").unwrap();

    let mut handle = start_fixture_watcher(&root, config()).unwrap();
    let batch = handle
        .recv_batch()
        .await
        .expect("watcher channel must remain open");
    assert_eq!(
        batch.changed_paths.into_iter().collect::<HashSet<_>>(),
        HashSet::from([kept, app.join(".gitignore")])
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn watcher_shutdown_releases_resources() {
    let tmp = tempfile::tempdir().unwrap();
    let handle = start_watcher(tmp.path(), WatcherConfig::default()).unwrap();
    handle.shutdown().await;
}
