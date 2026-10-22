//! Deterministic notification source for integration tests (Issue #4689).
//!
//! Prepare files before starting. All fixture files, including ignored ones,
//! enter the real debouncer in one raw event. No filtering or batching happens
//! here; the production pipeline remains responsible for both.

use std::{path::Path, sync::Mutex};

use notify::{Event, EventHandler, EventKind, RecursiveMode, Watcher, WatcherKind};

use super::{start_watcher_with, WatcherConfig, WatcherGuard, WatcherHandle};

/// Start the production delivery pipeline with a fixture notification source.
pub fn start_fixture_watcher(
    root: &Path,
    cfg: WatcherConfig,
) -> crate::error::Result<WatcherHandle> {
    start_watcher_with::<FixtureWatcher>(root, cfg, |debouncer| WatcherGuard::Fixture {
        _debouncer: debouncer,
    })
}

pub(super) struct FixtureWatcher {
    // Preserve the native handle's Send + Sync traits with test-support enabled.
    handler: Mutex<Box<dyn EventHandler>>,
}

impl Watcher for FixtureWatcher {
    fn new<F: EventHandler>(handler: F, _config: notify::Config) -> notify::Result<Self> {
        Ok(Self {
            handler: Mutex::new(Box::new(handler)),
        })
    }

    fn watch(&mut self, root: &Path, _mode: RecursiveMode) -> notify::Result<()> {
        let mut event = Event::new(EventKind::Any);
        let mut directories = vec![root.to_path_buf()];
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    directories.push(entry.path());
                } else {
                    event.paths.push(entry.path());
                }
            }
        }
        self.handler.get_mut().unwrap().handle_event(Ok(event));
        Ok(())
    }

    fn unwatch(&mut self, _path: &Path) -> notify::Result<()> {
        Ok(())
    }

    fn kind() -> WatcherKind {
        WatcherKind::NullWatcher
    }
}
