//! Explicit native OS delivery probe, excluded from default cargo test and hooks.
//! Run: cargo test -p gwt-core --test watcher_native -- --ignored

use std::{collections::HashSet, fs, time::Duration};

use gwt_core::index::watcher::{start_watcher, WatcherConfig};

#[tokio::test]
#[ignore = "native OS notification delivery is unbounded; run explicitly"]
async fn native_watcher_delivers_fifty_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = dunce::canonicalize(tmp.path()).unwrap();
    // test-hygiene: allow-native-watcher-deadline Explicit opt-in OS probe required by #4689 AC-2'; never a default gate.
    let mut handle = start_watcher(&root, WatcherConfig::default()).unwrap();
    let expected: HashSet<_> = (0..50)
        .map(|i| {
            let path = root.join(format!("f{i}.rs"));
            fs::write(&path, "// native\n").unwrap();
            path
        })
        .collect();
    let mut observed = HashSet::new();
    while observed.len() < expected.len() {
        let batch = tokio::time::timeout(Duration::from_secs(30), handle.recv_batch())
            .await
            .expect("native probe did not observe delivery within its diagnostic budget")
            .expect("watcher channel must remain open");
        // Native backends may also report the watched directory itself.
        observed.extend(batch.changed_paths.into_iter().filter(|path| {
            path.extension().and_then(|extension| extension.to_str()) == Some("rs")
        }));
    }
    assert_eq!(observed, expected);
    handle.shutdown().await;
}
