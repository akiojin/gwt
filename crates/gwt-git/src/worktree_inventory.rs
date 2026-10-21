//! One worktree inventory per Git common directory, shared by threads and
//! short-lived gwtd processes. Cache failures never turn a listing into empty
//! success, and notification latency cannot hide a completed Git mutation.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, OnceLock, TryLockError,
    },
    time::{Duration, SystemTime},
};

use gwt_core::{
    operation_deadline::{ensure_remaining, NamedFileLock},
    Result,
};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};

use crate::worktree::WorktreeInfo;

type Stamp = Vec<(PathBuf, Option<(SystemTime, u64)>)>;

#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    version: u8,
    stamp: Stamp,
    entries: Vec<WorktreeInfo>,
}

#[derive(Default)]
struct State {
    snapshot: Option<Snapshot>,
    watcher: Option<RecommendedWatcher>,
    watching_worktrees: bool,
}

#[derive(Default)]
struct Inventory {
    state: Mutex<State>,
    dirty: Arc<AtomicBool>,
}

static INVENTORIES: OnceLock<Mutex<HashMap<PathBuf, Arc<Inventory>>>> = OnceLock::new();
const SNAPSHOT: &str = "gwt-worktree-inventory-v1.json";

pub(super) fn list(
    repo_path: &Path,
    mut load: impl FnMut() -> Result<Vec<WorktreeInfo>>,
) -> Result<Vec<WorktreeInfo>> {
    ensure_remaining("worktree inventory")?;
    // These variables change Git's repository identity independently of the
    // supplied root. Preserve the original Git behavior without polluting a
    // filesystem-keyed inventory.
    if ["GIT_DIR", "GIT_COMMON_DIR", "GIT_WORK_TREE"]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        return load();
    }
    let Some(common) = gwt_core::repo_hash::repository_common_dir(repo_path) else {
        return load();
    };
    let inventory = {
        let mut inventories = INVENTORIES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        inventories.entry(common.clone()).or_default().clone()
    };
    let mut state = lock_state(&inventory)?;
    watch(&common, &inventory, &mut state);
    if inventory.dirty.swap(false, Ordering::AcqRel) {
        state.snapshot = None;
    }
    let Ok(stamp) = source_stamp(&common) else {
        // Without a reliable freshness proof, ask Git instead of serving old data.
        state.snapshot = None;
        return load();
    };
    if let Some(snapshot) = state.snapshot.as_ref().filter(|s| s.stamp == stamp) {
        ensure_remaining("worktree inventory")?;
        return Ok(snapshot.entries.clone());
    }
    let path = common.join(SNAPSHOT);
    if let Some(snapshot) = read_snapshot(&path, &stamp) {
        ensure_remaining("worktree inventory")?;
        let entries = snapshot.entries.clone();
        state.snapshot = Some(snapshot);
        return Ok(entries);
    }
    let lock = match NamedFileLock::acquire_quiet(
        &common.join("gwt-worktree-inventory.lock"),
        "worktree inventory",
    ) {
        Ok(lock) => lock,
        Err(error) => {
            ensure_remaining("worktree inventory")?;
            // Git can list a read-only repository; cache files are optional.
            tracing::debug!(%error, "worktree inventory persistence unavailable");
            return load();
        }
    };
    // Another process may have published while this caller waited for the lock.
    let mut stamp = source_stamp(&common)?;
    if let Some(snapshot) = read_snapshot(&path, &stamp) {
        ensure_remaining("worktree inventory")?;
        let entries = snapshot.entries.clone();
        state.snapshot = Some(snapshot);
        return Ok(entries);
    }
    loop {
        ensure_remaining("worktree inventory")?;
        let entries = load()?;
        let after = source_stamp(&common)?;
        if stamp != after {
            // Do not associate a listing from before a mutation with the stamp
            // after it. Waiters must observe one complete generation.
            stamp = after;
            continue;
        }
        ensure_remaining("worktree inventory")?;
        let snapshot = Snapshot {
            version: 1,
            stamp,
            entries: entries.clone(),
        };
        if let Err(error) = serde_json::to_vec(&snapshot)
            .map_err(io::Error::other)
            .and_then(|bytes| gwt_core::atomic_file::write_atomic(&path, &bytes))
        {
            tracing::debug!(%error, "worktree inventory snapshot not persisted");
        }
        state.snapshot = Some(snapshot);
        drop(lock);
        ensure_remaining("worktree inventory")?;
        return Ok(entries);
    }
}

fn lock_state(inventory: &Inventory) -> Result<MutexGuard<'_, State>> {
    loop {
        ensure_remaining("worktree inventory")?;
        match inventory.state.try_lock() {
            Ok(state) => return Ok(state),
            Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn read_snapshot(path: &Path, stamp: &Stamp) -> Option<Snapshot> {
    let snapshot: Snapshot = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (snapshot.version == 1 && snapshot.stamp == *stamp).then_some(snapshot)
}

fn source_stamp(common: &Path) -> io::Result<Stamp> {
    let worktrees = common.join("worktrees");
    let mut paths = vec![
        common.join("HEAD"),
        common.join("config"),
        worktrees.clone(),
    ];
    match fs::read_dir(&worktrees) {
        Ok(entries) => {
            for entry in entries {
                ensure_remaining("worktree inventory")?;
                let dir = entry?.path();
                paths.extend([dir.join("HEAD"), dir.join("gitdir"), dir.join("locked")]);
                match fs::read_to_string(dir.join("gitdir")) {
                    Ok(gitdir) => {
                        let gitdir = PathBuf::from(gitdir.trim());
                        paths.push(if gitdir.is_absolute() {
                            gitdir
                        } else {
                            dir.join(gitdir)
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let stamp = match fs::metadata(&path) {
                Ok(metadata) => Some((metadata.modified()?, metadata.len())),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            Ok((path, stamp))
        })
        .collect()
}

fn watch(common: &Path, inventory: &Inventory, state: &mut State) {
    let worktrees = common.join("worktrees");
    if state.watcher.is_none() {
        let root = common.to_path_buf();
        let dirty = inventory.dirty.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let changed = match event {
                Ok(event) => {
                    !matches!(event.kind, EventKind::Access(_))
                        && event.paths.iter().any(|path| {
                            path.starts_with(root.join("worktrees"))
                                || path == &root.join("HEAD")
                                || path == &root.join("config")
                        })
                }
                Err(_) => true,
            };
            if changed {
                dirty.store(true, Ordering::Release);
            }
        });
        if let Ok(mut watcher) = watcher {
            if watcher.watch(common, RecursiveMode::NonRecursive).is_ok() {
                state.watcher = Some(watcher);
            }
        }
    }
    if !worktrees.is_dir() {
        state.watching_worktrees = false;
    } else if !state.watching_worktrees {
        if let Some(watcher) = &mut state.watcher {
            state.watching_worktrees = watcher.watch(&worktrees, RecursiveMode::Recursive).is_ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_notifications_invalidate_the_inventory() {
        let common = tempfile::tempdir().unwrap();
        let inventory = Inventory::default();
        let mut state = State::default();
        watch(common.path(), &inventory, &mut state);
        fs::write(common.path().join("HEAD"), "ref: refs/heads/main\n").unwrap();
        // test-hygiene: allow-wall-clock-deadline AC-1 requires observing the real OS notification; this only bounds a stuck integration probe.
        let deadline = std::time::Instant::now() + gwt_core::deadline_budget::HANG_GUARD;
        while !inventory.dirty.load(Ordering::Acquire) {
            assert!(
                // test-hygiene: allow-wall-clock-deadline Native notification delivery has no injectable completion; the shared hang guard detects a stuck probe.
                std::time::Instant::now() < deadline,
                "no native HEAD notification"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let worktrees = common.path().join("worktrees");
        fs::create_dir(&worktrees).unwrap();
        fs::create_dir(worktrees.join("linked")).unwrap();
        watch(common.path(), &inventory, &mut state);
        assert!(state.watching_worktrees);
        inventory.dirty.store(false, Ordering::Release);
        fs::write(worktrees.join("linked/locked"), "").unwrap();
        while !inventory.dirty.load(Ordering::Acquire) {
            assert!(
                // test-hygiene: allow-wall-clock-deadline AC-1 also requires recursive native delivery; the guard never asserts notification speed.
                std::time::Instant::now() < deadline,
                "no recursive worktree notification"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
