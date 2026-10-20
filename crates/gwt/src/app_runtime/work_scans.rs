//! Filesystem inputs for the Workspace background scans.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::SystemTime,
};

use notify::Watcher;

#[derive(Default)]
pub(super) struct WorkScanInputs {
    roots: Option<WatchRoots>,
    watcher: Option<notify::RecommendedWatcher>,
    revision: Arc<AtomicU64>,
    healthy: Arc<AtomicBool>,
}

impl WorkScanInputs {
    /// Subscribe before measuring Git, then compare `current_revision` after
    /// measuring. Missing roots and unreliable notifications disable reuse.
    pub(super) fn observe(
        &mut self,
        project_root: &Path,
        worktree_paths: &[PathBuf],
    ) -> Option<u64> {
        let Some(roots) = WatchRoots::capture(project_root, worktree_paths) else {
            self.clear();
            return None;
        };
        if self.roots.as_ref() != Some(&roots) || self.current_revision().is_none() {
            self.clear();
            self.revision.fetch_add(1, Ordering::SeqCst);
            // A late callback from the previous subscription must not poison
            // the replacement watch's health.
            self.healthy = Arc::new(AtomicBool::new(true));
            let revision = Arc::clone(&self.revision);
            let healthy = Arc::clone(&self.healthy);
            let watched = roots.clone();
            let watcher = notify::recommended_watcher(move |event| {
                record_change(event, &watched, &revision, &healthy);
            });
            let Ok(mut watcher) = watcher else {
                self.clear();
                return None;
            };
            for stamp in &roots.stamps {
                // A parent recursive watch already covers nested Git dirs.
                if roots
                    .stamps
                    .iter()
                    .any(|other| other.path != stamp.path && stamp.path.starts_with(&other.path))
                {
                    continue;
                }
                if watcher
                    .watch(&stamp.path, notify::RecursiveMode::Recursive)
                    .is_err()
                {
                    self.clear();
                    return None;
                }
            }
            self.roots = Some(roots);
            self.watcher = Some(watcher);
        }
        self.current_revision()
    }

    pub(super) fn current_revision(&self) -> Option<u64> {
        let revision = self.revision.load(Ordering::SeqCst);
        (self.watcher.is_some() && self.healthy.load(Ordering::SeqCst)).then_some(revision)
    }

    fn clear(&mut self) {
        self.healthy.store(false, Ordering::SeqCst);
        self.watcher = None;
        self.roots = None;
    }
}

#[derive(Clone, PartialEq, Eq)]
struct WatchRoots {
    git: Vec<PathBuf>,
    worktrees: Vec<PathBuf>,
    stamps: Vec<RootStamp>,
}

impl WatchRoots {
    fn capture(project_root: &Path, worktree_paths: &[PathBuf]) -> Option<Self> {
        // This resolver reads .git/commondir metadata without launching Git.
        let common = gwt_core::repo_hash::repository_common_dir(project_root)?;
        let mut git = vec![common];
        let mut worktrees = Vec::new();
        for path in worktree_paths {
            let worktree = dunce::canonicalize(path).ok()?;
            let dot_git = worktree.join(".git");
            if dot_git.is_file() {
                // Usually below common/worktrees/, but an explicit gitdir may
                // live elsewhere and its index must still invalidate status.
                let contents = fs::read_to_string(&dot_git).ok()?;
                let gitdir = contents
                    .lines()
                    .find_map(|line| line.trim().strip_prefix("gitdir:"))?
                    .trim();
                if gitdir.is_empty() {
                    return None;
                }
                let gitdir = PathBuf::from(gitdir);
                git.push(if gitdir.is_absolute() {
                    gitdir
                } else {
                    worktree.join(gitdir)
                });
            }
            worktrees.push(worktree);
        }
        git = git
            .into_iter()
            .map(dunce::canonicalize)
            .collect::<Result<_, _>>()
            .ok()?;
        git.sort();
        git.dedup();
        worktrees.sort();
        worktrees.dedup();
        let mut paths = git.iter().chain(&worktrees).cloned().collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        let stamps = paths
            .into_iter()
            .map(RootStamp::capture)
            .collect::<Option<_>>()?;
        Some(Self {
            git,
            worktrees,
            stamps,
        })
    }

    fn relevant(&self, path: &Path) -> bool {
        if self.git.iter().any(|root| root.starts_with(path)) {
            return true;
        }
        if self.git.iter().any(|root| path.starts_with(root)) {
            // Git status creates and removes index.lock even when its input
            // and final index are unchanged. Atomic updates still notify the
            // destination (index, packed-refs or a ref), which remains relevant.
            return !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".lock"));
        }
        self.worktrees.iter().any(|root| {
            if root.starts_with(path) {
                return true;
            }
            path.strip_prefix(root).is_ok_and(|relative| {
                !gwt_git::worktree::status_entry_is_gwt_runtime_write(
                    &relative.to_string_lossy().replace('\\', "/"),
                )
            })
        })
    }
}

/// Root replacement can detach a native watch even when its path is unchanged.
#[derive(Clone, PartialEq, Eq)]
struct RootStamp {
    path: PathBuf,
    created: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl RootStamp {
    fn capture(path: PathBuf) -> Option<Self> {
        let metadata = fs::metadata(&path).ok()?;
        if !metadata.is_dir() {
            return None;
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Some(Self {
            path,
            created: metadata.created().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        })
    }
}

fn record_change(
    event: notify::Result<notify::Event>,
    roots: &WatchRoots,
    revision: &AtomicU64,
    healthy: &AtomicBool,
) {
    let changed = match event {
        Ok(event) if event.kind.is_access() => false,
        // Windows reports directory timestamp/attribute refreshes caused by
        // read-only Git commands as Modify(Any). Inputs change through file
        // writes or entry create/remove/rename events, which remain relevant.
        Ok(event)
            if event.kind == notify::EventKind::Modify(notify::event::ModifyKind::Any)
                && !event.need_rescan()
                && !event.paths.is_empty()
                && event.paths.iter().all(|path| path.is_dir()) =>
        {
            false
        }
        Ok(event) => {
            if event.need_rescan() {
                healthy.store(false, Ordering::SeqCst);
            }
            if matches!(
                event.kind,
                notify::EventKind::Remove(_)
                    | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
            ) && event
                .paths
                .iter()
                .any(|path| roots.stamps.iter().any(|root| root.path.starts_with(path)))
            {
                healthy.store(false, Ordering::SeqCst);
            }
            event.need_rescan()
                || event.paths.is_empty()
                || event.paths.iter().any(|path| roots.relevant(path))
        }
        Err(_) => {
            healthy.store(false, Ordering::SeqCst);
            true
        }
    };
    if changed {
        revision.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };

    use super::*;

    fn repository(root: &Path) -> PathBuf {
        let repo = root.join("repo");
        fs::create_dir_all(repo.join(".git/refs/heads")).unwrap();
        fs::write(repo.join(".git/config"), "[core]\n\tbare = false\n").unwrap();
        fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(repo.join(".git/refs/heads/main"), "first\n").unwrap();
        dunce::canonicalize(repo).unwrap()
    }

    fn wait_for_change(inputs: &WorkScanInputs, previous: u64) -> u64 {
        // test-hygiene: allow-wall-clock-deadline Real OS notify integration must prove delivery; this only bounds a hung probe.
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(revision) = inputs
                .current_revision()
                .filter(|revision| *revision != previous)
            {
                return revision;
            }
            assert!(
                // test-hygiene: allow-wall-clock-deadline Real OS delivery liveness bound, not a latency or ordering assertion.
                Instant::now() < deadline,
                "filesystem change did not invalidate the scan"
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn unchanged_inputs_reuse_the_revision_without_git() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(temp.path());
        let mut inputs = WorkScanInputs::default();
        assert_eq!(inputs.current_revision(), None);
        let before = gwt_core::process::thread_git_spawn_count();
        let first = inputs
            .observe(&repo, std::slice::from_ref(&repo))
            .expect("watch repository");
        assert_eq!(
            inputs.observe(&repo, std::slice::from_ref(&repo)),
            Some(first)
        );
        assert_eq!(inputs.current_revision(), Some(first));
        assert_eq!(gwt_core::process::thread_git_spawn_count(), before);
    }

    #[test]
    fn refs_linked_index_and_worktree_edits_invalidate_the_revision() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(temp.path());
        let linked = temp.path().join("linked");
        fs::create_dir_all(&linked).unwrap();
        let git_dir = repo.join(".git/worktrees/linked");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .unwrap();
        fs::write(git_dir.join("commondir"), "../..\n").unwrap();
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        for path in [
            repo.join(".git/refs/heads/main"),
            git_dir.join("index"),
            linked.join("source.txt"),
        ] {
            let mut inputs = WorkScanInputs::default();
            let revision = inputs
                .observe(&linked, std::slice::from_ref(&linked))
                .expect("watch linked tree");
            fs::write(path, "updated\n").unwrap();
            wait_for_change(&inputs, revision);
        }
    }

    #[test]
    fn notifications_ignore_reads_and_runtime_writes_and_reject_errors() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(temp.path());
        let roots = WatchRoots::capture(&repo, std::slice::from_ref(&repo)).unwrap();
        let revision = AtomicU64::new(0);
        let healthy = AtomicBool::new(true);
        for event in [
            notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
                .add_path(repo.join(".git/refs/heads")),
            notify::Event::new(notify::EventKind::Access(notify::event::AccessKind::Any))
                .add_path(repo.join("source.txt")),
            notify::Event::new(notify::EventKind::Any)
                .add_path(repo.join(".gwt/work/events/new.jsonl")),
            notify::Event::new(notify::EventKind::Create(notify::event::CreateKind::File))
                .add_path(repo.join(".git/index.lock")),
            notify::Event::new(notify::EventKind::Remove(notify::event::RemoveKind::File))
                .add_path(repo.join(".git/index.lock")),
        ] {
            record_change(Ok(event), &roots, &revision, &healthy);
        }
        assert_eq!(revision.load(Ordering::SeqCst), 0);
        record_change(
            Ok(notify::Event::new(notify::EventKind::Any)
                .add_path(repo.join(".git/refs/heads/main"))),
            &roots,
            &revision,
            &healthy,
        );
        assert_eq!(revision.load(Ordering::SeqCst), 1);
        record_change(
            Ok(
                notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Name(
                    notify::event::RenameMode::Both,
                )))
                .add_path(repo.join(".git/index.lock"))
                .add_path(repo.join(".git/index")),
            ),
            &roots,
            &revision,
            &healthy,
        );
        assert_eq!(revision.load(Ordering::SeqCst), 2);
        record_change(
            Err(notify::Error::generic("lost filesystem notifications")),
            &roots,
            &revision,
            &healthy,
        );
        assert!(!healthy.load(Ordering::SeqCst));
        assert_eq!(revision.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn missing_roots_disable_reuse_and_recreation_recovers() {
        let temp = tempfile::tempdir().unwrap();
        let repo = repository(temp.path());
        let linked = temp.path().join("linked");
        fs::create_dir_all(&linked).unwrap();
        let mut inputs = WorkScanInputs::default();
        let first = inputs
            .observe(&repo, std::slice::from_ref(&linked))
            .expect("watch roots");
        fs::remove_dir(&linked).unwrap();
        assert_eq!(inputs.observe(&repo, std::slice::from_ref(&linked)), None);
        assert_eq!(inputs.current_revision(), None);
        fs::create_dir(&linked).unwrap();
        let recovered = inputs
            .observe(&repo, std::slice::from_ref(&linked))
            .expect("watch recreated tree");
        assert_ne!(recovered, first);
        assert_eq!(inputs.observe(&repo, &[linked]), Some(recovered));
        assert_eq!(inputs.observe(&temp.path().join("missing"), &[]), None);
    }
}
