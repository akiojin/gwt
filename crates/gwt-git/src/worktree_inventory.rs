//! Process-shared, repository-scoped worktree listings (Issue #5377).
//!
//! Notify invalidates a listing when Git's worktree registry changes. A
//! synchronous registry signature also closes the notification-delivery gap:
//! the very next lookup after an add/remove cannot reuse the old listing.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::SystemTime,
};

use gwt_core::{GwtError, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::worktree::WorktreeInfo;

type FileStamp = Option<(SystemTime, u64)>;
type RegistryStamp = Vec<(PathBuf, FileStamp)>;

struct CachedListing {
    registry: RegistryStamp,
    generation: u64,
    paths_exist: Vec<bool>,
    entries: Vec<WorktreeInfo>,
}

struct Inventory {
    generation: Arc<AtomicU64>,
    healthy: Arc<AtomicBool>,
    state: Mutex<InventoryState>,
}

struct InventoryState {
    watcher: Option<RecommendedWatcher>,
    watching_registry: bool,
    cached: Option<CachedListing>,
}

static INVENTORIES: OnceLock<Mutex<HashMap<PathBuf, Arc<Inventory>>>> = OnceLock::new();

fn invalidates_registry(event: &Event, common: &Path) -> bool {
    !matches!(event.kind, EventKind::Access(_))
        && (event.need_rescan()
            || event.paths.iter().any(|path| {
                path == &common.join("HEAD")
                    || path == &common.join("worktrees")
                    || path
                        .strip_prefix(common.join("worktrees"))
                        .is_ok_and(|relative| {
                            relative.components().count() == 1
                                || relative.file_name().is_some_and(|name| {
                                    matches!(
                                        name.to_str(),
                                        Some("HEAD" | "gitdir" | "locked" | "commondir")
                                    )
                                })
                        })
            }))
}

impl Inventory {
    fn new(common: &Path) -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let healthy = Arc::new(AtomicBool::new(true));
        let watched_common = common.to_path_buf();
        let changed = generation.clone();
        let watch_health = healthy.clone();
        let watcher =
            notify::recommended_watcher(move |result: notify::Result<Event>| {
                #[cfg(test)]
                eprintln!("inventory notify common={watched_common:?} result={result:?}");
                match result {
                Ok(event) if invalidates_registry(&event, &watched_common) => {
                    changed.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => {
                    watch_health.store(false, Ordering::SeqCst);
                }
                _ => {}
                }
            })
            .and_then(|mut watcher| {
                watcher.watch(common, RecursiveMode::NonRecursive)?;
                Ok(watcher)
            })
            .ok();
        if watcher.is_none() {
            healthy.store(false, Ordering::SeqCst);
        }
        Self {
            generation,
            healthy,
            state: Mutex::new(InventoryState {
                watcher,
                watching_registry: false,
                cached: None,
            }),
        }
    }
}

fn file_stamp(path: &Path) -> io::Result<FileStamp> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some((metadata.modified()?, metadata.len()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn registry_stamp(common: &Path) -> io::Result<RegistryStamp> {
    let registry = common.join("worktrees");
    let mut directories = match fs::read_dir(&registry) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    directories.sort();
    let mut stamp = vec![(common.join("HEAD"), file_stamp(&common.join("HEAD"))?)];
    for directory in directories {
        // The path itself retains topology even for a partially created
        // registry entry, before Git writes its metadata files.
        stamp.push((directory.clone(), None));
        for name in ["HEAD", "gitdir", "locked", "commondir"] {
            let path = directory.join(name);
            stamp.push((path.clone(), file_stamp(&path)?));
        }
    }
    Ok(stamp)
}

pub(crate) fn list(
    repo: &Path,
    mut load: impl FnMut() -> Result<Vec<WorktreeInfo>>,
) -> Result<Vec<WorktreeInfo>> {
    let Some(common) = gwt_core::repo_hash::repository_common_dir(repo) else {
        return load();
    };
    let inventory = {
        let mut inventories = INVENTORIES
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| GwtError::Git("worktree inventory map poisoned".into()))?;
        inventories
            .entry(common.clone())
            .or_insert_with(|| Arc::new(Inventory::new(&common)))
            .clone()
    };
    // Only this repository is serialized. The lock spans the actual Git
    // command so simultaneous misses collapse to one successful listing.
    let mut state = loop {
        gwt_core::operation_deadline::ensure_remaining("worktree inventory")
            .map_err(|error| GwtError::Git(format!("worktree list: {error}")))?;
        match inventory.state.try_lock() {
            Ok(state) => break state,
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(GwtError::Git("worktree inventory poisoned".into()));
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    };
    let registry = common.join("worktrees");
    let registry_exists = registry.is_dir();
    if registry_exists != state.watching_registry {
        if let Some(watcher) = state.watcher.as_mut() {
            if registry_exists {
                if watcher.watch(&registry, RecursiveMode::Recursive).is_err() {
                    inventory.healthy.store(false, Ordering::SeqCst);
                } else {
                    state.watching_registry = true;
                }
            } else {
                let _ = watcher.unwatch(&registry);
                state.watching_registry = false;
            }
        }
    }
    // Never trust stale data when monitoring or filesystem inspection fails.
    if !inventory.healthy.load(Ordering::SeqCst) {
        state.cached = None;
        return load();
    }
    let stamp = match registry_stamp(&common) {
        Ok(stamp) => stamp,
        Err(_) => {
            state.cached = None;
            return load();
        }
    };
    let generation = inventory.generation.load(Ordering::SeqCst);
    if let Some(cached) = &state.cached {
        if cached.generation == generation
            && cached.registry == stamp
            && cached
                .entries
                .iter()
                .zip(&cached.paths_exist)
                .all(|(entry, existed)| entry.path.exists() == *existed)
        {
            return Ok(cached.entries.clone());
        }
    }
    state.cached = None;
    // Recheck after loading: a concurrent add/remove must not publish a
    // listing captured before its registry change. Continuous mutation fails
    // closed rather than returning a known-obsolete inventory.
    let mut before = stamp;
    for _ in 0..3 {
        let generation = inventory.generation.load(Ordering::SeqCst);
        let entries = load()?;
        let after = registry_stamp(&common)
            .map_err(|error| GwtError::Git(format!("worktree registry: {error}")))?;
        if before == after {
            state.cached = Some(CachedListing {
                registry: after,
                generation,
                paths_exist: entries.iter().map(|entry| entry.path.exists()).collect(),
                entries: entries.clone(),
            });
            return Ok(entries);
        }
        before = after;
    }
    Err(GwtError::Git(
        "worktree registry changed during every listing; retry".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_ignore_index_churn_and_invalidate_registry_metadata() {
        let common = Path::new("repo/.git");
        let event = |path: &str| {
            Event::new(EventKind::Modify(notify::event::ModifyKind::Any))
                .add_path(common.join(path))
        };
        assert!(invalidates_registry(
            &event("worktrees/feature/HEAD"),
            common
        ));
        assert!(invalidates_registry(
            &event("worktrees/feature/locked"),
            common
        ));
        assert!(invalidates_registry(&event("worktrees/feature"), common));
        assert!(invalidates_registry(&event("worktrees"), common));
        assert!(!invalidates_registry(
            &event("worktrees/feature/index"),
            common
        ));
        assert!(!invalidates_registry(
            &event("objects/pack/file.pack"),
            common
        ));
    }

    #[test]
    fn native_notification_invalidates_when_first_registry_is_created() {
        let common = tempfile::tempdir().unwrap();
        let inventory = Inventory::new(common.path());
        assert!(
            inventory.healthy.load(Ordering::SeqCst),
            "native watcher available"
        );
        fs::create_dir(common.path().join("worktrees")).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while inventory.generation.load(Ordering::SeqCst) == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "registry creation notification"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}
