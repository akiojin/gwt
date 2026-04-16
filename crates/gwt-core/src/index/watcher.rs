//! Per-Worktree filesystem watcher with debounce and batching.
//!
//! Wraps `notify-debouncer-mini` to:
//! - Honor `.gitignore` rules via the `ignore` crate
//! - Debounce bursts of events for `WatcherConfig::debounce` (default 2 s)
//! - Split batches at `WatcherConfig::batch_limit` paths (default 100)
//!
//! The watcher does NOT trigger ChromaDB writes itself; consumers
//! drain `WatcherHandle::recv_batch()` and dispatch the appropriate
//! `runner index-* --mode incremental` job per batch.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use tokio::sync::mpsc;

use crate::error::{GwtError, Result};

/// Tunable parameters for `start_watcher`.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    pub debounce: Duration,
    pub batch_limit: usize,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_secs(2),
            batch_limit: 100,
        }
    }
}

/// One debounced batch of changed paths.
#[derive(Debug, Clone)]
pub struct WatcherBatch {
    pub changed_paths: Vec<PathBuf>,
}

/// Handle returned from `start_watcher`. Drop or call `shutdown()` to stop.
pub struct WatcherHandle {
    rx: mpsc::Receiver<WatcherBatch>,
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    _shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    forwarder: Option<tokio::task::JoinHandle<()>>,
}

impl WatcherHandle {
    /// Receive the next batch. Returns `None` if the watcher has been shut
    /// down or the inner channel has closed.
    pub async fn recv_batch(&mut self) -> Option<WatcherBatch> {
        self.rx.recv().await
    }

    /// Stop the watcher and release resources.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self._shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.forwarder.take() {
            let _ = handle.await;
        }
        // Debouncer Drop releases notify resources.
    }
}

/// Start a per-Worktree watcher rooted at `worktree_path`. Returns a handle
/// the caller can poll for batches.
pub fn start_watcher(worktree_path: &Path, cfg: WatcherConfig) -> Result<WatcherHandle> {
    if !worktree_path.is_dir() {
        return Err(GwtError::Other(format!(
            "worktree path is not a directory: {}",
            worktree_path.display()
        )));
    }

    // Canonicalize the worktree path so that filesystem events (which `notify`
    // delivers using the canonical form on macOS — `/var` → `/private/var`)
    // share the same prefix when we run gitignore filtering.
    let worktree_path_owned =
        dunce::canonicalize(worktree_path).unwrap_or_else(|_| worktree_path.to_path_buf());
    let worktree_path = worktree_path_owned.as_path();

    let gitignore = build_gitignore(worktree_path);

    // Bridge sync notify callback → tokio mpsc.
    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<Vec<PathBuf>>();
    let mut debouncer: Debouncer<notify::RecommendedWatcher> =
        new_debouncer(cfg.debounce, move |res: DebounceEventResult| {
            if let Ok(events) = res {
                let paths: Vec<PathBuf> = events.into_iter().map(|e| e.path).collect();
                if !paths.is_empty() {
                    let _ = raw_tx.send(paths);
                }
            }
        })
        .map_err(|e| GwtError::Other(format!("debouncer init: {e}")))?;

    debouncer
        .watcher()
        .watch(worktree_path, RecursiveMode::Recursive)
        .map_err(|e| GwtError::Other(format!("watch path: {e}")))?;

    let (tx, rx) = mpsc::channel::<WatcherBatch>(64);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let worktree_owned = worktree_path.to_path_buf();
    let batch_limit = cfg.batch_limit;

    let forwarder = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }

            let mut accumulated: Vec<PathBuf> = Vec::new();
            // Drain everything currently in the sync channel non-blockingly.
            while let Ok(paths) = raw_rx.try_recv() {
                accumulated.extend(paths);
            }
            if accumulated.is_empty() {
                continue;
            }

            // Filter through gitignore.
            let filtered: Vec<PathBuf> = accumulated
                .into_iter()
                .filter(|p| !is_ignored(&gitignore, &worktree_owned, p))
                .collect();
            if filtered.is_empty() {
                continue;
            }

            // Split into ≤batch_limit chunks and emit each as its own batch.
            for chunk in filtered.chunks(batch_limit) {
                let batch = WatcherBatch {
                    changed_paths: chunk.to_vec(),
                };
                if tx.send(batch).await.is_err() {
                    return;
                }
            }
        }
    });

    Ok(WatcherHandle {
        rx,
        _debouncer: debouncer,
        _shutdown_tx: Some(shutdown_tx),
        forwarder: Some(forwarder),
    })
}

/// Prefixes under the Worktree root that should never feed into the index
/// even when they are not listed in `.gitignore`. These mirror the
/// runner's `classify_file_bucket` skip list plus common heavy build
/// artifact directories so the watcher does not trigger rebuilds from
/// agent hook writes or cargo target churn.
const WATCHER_BUILTIN_SKIP_PREFIXES: &[&str] = &[
    ".git",
    ".claude",
    ".codex",
    ".gemini",
    ".gwt",
    "tasks",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".nuxt",
];

fn build_gitignore(worktree: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(worktree);
    let gitignore_path = worktree.join(".gitignore");
    if gitignore_path.is_file() {
        let _ = builder.add(&gitignore_path);
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

fn is_builtin_skip(worktree: &Path, path: &Path) -> bool {
    let rel = path.strip_prefix(worktree).unwrap_or(path);
    let first = rel.components().next().and_then(|c| c.as_os_str().to_str());
    match first {
        Some(name) => WATCHER_BUILTIN_SKIP_PREFIXES.contains(&name),
        None => false,
    }
}

fn is_ignored(gi: &Gitignore, worktree: &Path, path: &Path) -> bool {
    if is_builtin_skip(worktree, path) {
        return true;
    }
    let rel = path.strip_prefix(worktree).unwrap_or(path);
    let is_dir = path.is_dir();
    // Use matched_path_or_any_parents so a `ignored/` rule excludes
    // every file underneath `ignored/`, not just the directory entry itself.
    matches!(
        gi.matched_path_or_any_parents(rel, is_dir),
        ignore::Match::Ignore(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let cfg = WatcherConfig::default();
        assert_eq!(cfg.debounce, Duration::from_secs(2));
        assert_eq!(cfg.batch_limit, 100);
    }

    #[test]
    fn rejects_non_directory() {
        let result = start_watcher(Path::new("/nonexistent/path/xyz"), WatcherConfig::default());
        assert!(result.is_err());
    }

    #[test]
    fn removed_specs_archive_prefix_is_not_builtin_skip() {
        let root = Path::new("/repo");
        let path = root.join("specs-archive/SPEC-10/spec.md");

        assert!(!is_builtin_skip(root, &path));
    }
}
