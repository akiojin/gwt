//! Repo-local coordination watcher for the Board tab.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::Duration;

use gwt_core::coordination::{
    coordination_dir, ensure_repo_local_files, load_snapshot, CoordinationSnapshot,
    BOARD_PROJECTION_FILE_NAME, CARDS_PROJECTION_FILE_NAME,
};
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;

/// Full board snapshot packet sent from the watcher thread into the UI loop.
#[derive(Debug, Clone)]
pub enum BoardWatcherPacket {
    SetSnapshot(CoordinationSnapshot),
}

/// Public watcher handle kept alive for the lifetime of the TUI.
pub struct BoardWatcherHandle {
    _thread: thread::JoinHandle<()>,
}

pub fn spawn(worktree_root: PathBuf, tx: Sender<BoardWatcherPacket>) -> BoardWatcherHandle {
    let handle = thread::Builder::new()
        .name("gwt-board-watcher".into())
        .spawn(move || run(worktree_root, tx))
        .expect("spawn board watcher thread");
    BoardWatcherHandle { _thread: handle }
}

fn run(worktree_root: PathBuf, tx: Sender<BoardWatcherPacket>) {
    if let Err(err) = ensure_repo_local_files(&worktree_root) {
        tracing::warn!(
            target: "gwt_tui::board_watcher",
            repo = %worktree_root.display(),
            error = %err,
            "failed to initialize coordination storage"
        );
        return;
    }

    if publish_snapshot(&worktree_root, &tx).is_err() {
        return;
    }

    let watch_dir = coordination_dir(&worktree_root);
    let (evt_tx, evt_rx) = channel();
    let mut debouncer = match new_debouncer(Duration::from_millis(150), evt_tx) {
        Ok(debouncer) => debouncer,
        Err(err) => {
            tracing::warn!(
                target: "gwt_tui::board_watcher",
                repo = %worktree_root.display(),
                error = %err,
                "failed to initialize board watcher"
            );
            return;
        }
    };

    if let Err(err) = debouncer
        .watcher()
        .watch(&watch_dir, RecursiveMode::NonRecursive)
    {
        tracing::warn!(
            target: "gwt_tui::board_watcher",
            dir = %watch_dir.display(),
            error = %err,
            "failed to watch coordination directory"
        );
        return;
    }

    while let Ok(events) = evt_rx.recv() {
        let Ok(events) = events else {
            continue;
        };
        if !events.iter().any(|event| is_projection_path(&event.path)) {
            continue;
        }
        if publish_snapshot(&worktree_root, &tx).is_err() {
            return;
        }
    }
}

fn publish_snapshot(
    worktree_root: &std::path::Path,
    tx: &Sender<BoardWatcherPacket>,
) -> Result<(), ()> {
    match load_snapshot(worktree_root) {
        Ok(snapshot) => tx
            .send(BoardWatcherPacket::SetSnapshot(snapshot))
            .map_err(|_| ()),
        Err(err) => {
            tracing::warn!(
                target: "gwt_tui::board_watcher",
                repo = %worktree_root.display(),
                error = %err,
                "failed to load coordination snapshot"
            );
            Ok(())
        }
    }
}

fn is_projection_path(path: &std::path::Path) -> bool {
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some(BOARD_PROJECTION_FILE_NAME | CARDS_PROJECTION_FILE_NAME)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_projection_path_matches_only_latest_snapshots() {
        assert!(is_projection_path(std::path::Path::new(
            "board.latest.json"
        )));
        assert!(is_projection_path(std::path::Path::new(
            "cards.latest.json"
        )));
        assert!(!is_projection_path(std::path::Path::new("events.jsonl")));
        assert!(!is_projection_path(std::path::Path::new(".lock")));
    }
}
