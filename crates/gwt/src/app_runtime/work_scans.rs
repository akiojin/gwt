//! Filesystem inputs for the Workspace background scans.

use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::SystemTime,
};

use notify::Watcher;
use sha2::{Digest, Sha256};

#[derive(Default)]
pub(super) struct WorkScanInputs {
    roots: Option<WatchRoots>,
    watcher: Option<notify::RecommendedWatcher>,
    revision: Arc<AtomicU64>,
    healthy: Arc<AtomicBool>,
    external: Option<ExternalGitInputs>,
}

impl WorkScanInputs {
    /// Subscribe before measuring Git, then compare `current_revision` after
    /// measuring. Missing roots and unreliable notifications disable reuse.
    pub(super) fn observe(
        &mut self,
        project_root: &Path,
        worktree_paths: &[PathBuf],
    ) -> Option<u64> {
        self.observe_roots(project_root, worktree_paths)?;
        let roots = self.roots.as_ref()?;
        let repository = if gwt_core::paths::git_repository_discovery_possible(project_root) {
            dunce::canonicalize(project_root).ok()?
        } else {
            gwt_core::repo_hash::repository_common_dir(project_root)?
        };
        let mut contexts = vec![repository.clone()];
        contexts.extend(
            roots
                .worktrees
                .iter()
                .filter(|path| **path != repository)
                .cloned(),
        );
        if !self
            .external
            .as_ref()
            .is_some_and(|inputs| inputs.contexts == contexts && inputs.is_current())
        {
            let Some(inputs) = ExternalGitInputs::discover(contexts, roots) else {
                self.clear();
                return None;
            };
            self.external = Some(inputs);
            self.revision.fetch_add(1, Ordering::SeqCst);
        }
        self.current_revision()
    }

    fn observe_roots(&mut self, project_root: &Path, worktree_paths: &[PathBuf]) -> Option<u64> {
        let Some(roots) = WatchRoots::capture(project_root, worktree_paths) else {
            self.clear();
            return None;
        };
        if self.roots.as_ref() != Some(&roots) || self.watcher_revision().is_none() {
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
        self.watcher_revision()
    }

    pub(super) fn current_revision(&self) -> Option<u64> {
        if self
            .external
            .as_ref()
            .is_some_and(|inputs| !inputs.is_current())
        {
            return None;
        }
        self.watcher_revision()
    }

    fn watcher_revision(&self) -> Option<u64> {
        let revision = self.revision.load(Ordering::SeqCst);
        (self.watcher.is_some() && self.healthy.load(Ordering::SeqCst)).then_some(revision)
    }

    fn clear(&mut self) {
        self.healthy.store(false, Ordering::SeqCst);
        self.watcher = None;
        self.roots = None;
        self.external = None;
    }
}

/// Git discovers its own includes and path interpolation once. Subsequent
/// observations read those small files, including paths that do not yet exist.
struct ExternalGitInputs {
    contexts: Vec<PathBuf>,
    environment: Vec<(OsString, OsString)>,
    files: Vec<(PathBuf, Option<[u8; 32]>)>,
}

impl ExternalGitInputs {
    fn discover(contexts: Vec<PathBuf>, roots: &WatchRoots) -> Option<Self> {
        let environment = git_input_environment();
        let repository = contexts.first()?;
        let mut paths = BTreeSet::new();
        let variables = git_input_output(repository, &["var", "-l"])?;
        let mut global_locations_known = false;
        for line in variables.lines() {
            if let Some((key, value)) = line.split_once('=') {
                global_locations_known |= key == "GIT_CONFIG_GLOBAL";
                if matches!(
                    key,
                    "GIT_CONFIG_SYSTEM"
                        | "GIT_CONFIG_GLOBAL"
                        | "GIT_ATTR_SYSTEM"
                        | "GIT_ATTR_GLOBAL"
                ) {
                    paths.insert(input_path(repository, value));
                }
            }
        }
        // Older Git versions do not expose absent global config locations.
        // Without those paths, later creation could silently invalidate status.
        if !global_locations_known {
            return None;
        }
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.map(|home| PathBuf::from(home).join(".config")))
        {
            paths.insert(config_home.join("git/ignore"));
        }
        // Branch-name includeIf rules and newly created worktree configs can
        // introduce external inputs even if no previous section named them.
        for root in &roots.git {
            paths.extend([
                root.join("HEAD"),
                root.join("config"),
                root.join("config.worktree"),
            ]);
        }
        let mut confirmed_paths = paths.clone();
        discover_all_config_inputs(&contexts, &mut paths)?;
        let files = paths
            .iter()
            .map(|path| file_fingerprint(path).map(|fingerprint| (path.clone(), fingerprint)))
            .collect::<Option<_>>()?;
        // A config may introduce a new include between the first query and
        // the byte capture. Confirm the dependency graph before trusting it.
        discover_all_config_inputs(&contexts, &mut confirmed_paths)?;
        if paths != confirmed_paths {
            return None;
        }
        let inputs = Self {
            contexts,
            environment,
            files,
        };
        inputs.is_current().then_some(inputs)
    }

    fn is_current(&self) -> bool {
        self.environment == git_input_environment()
            && self
                .files
                .iter()
                .all(|(path, previous)| file_fingerprint(path).as_ref() == Some(previous))
    }
}

fn discover_all_config_inputs(contexts: &[PathBuf], paths: &mut BTreeSet<PathBuf>) -> Option<()> {
    if discover_config_inputs(contexts.first()?, contexts, paths)? {
        for worktree in contexts.iter().skip(1) {
            discover_config_inputs(worktree, contexts, paths)?;
        }
    }
    Some(())
}

fn discover_config_inputs(
    repository: &Path,
    contexts: &[PathBuf],
    paths: &mut BTreeSet<PathBuf>,
) -> Option<bool> {
    // Unlike --list, --get-regexp applies --type=path, so Git expands ~/,
    // ~user/ and %(prefix) paths without a second config parser here.
    let output = git_input_output(
        repository,
        &[
            "config",
            "--null",
            "--show-origin",
            "--includes",
            "--type=path",
            "--get-regexp",
            ".*",
        ],
    )?;
    let mut entries = output.split_terminator('\0');
    let mut conditional = false;
    while let Some(origin) = entries.next() {
        let entry = entries.next()?;
        let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
        let origin = origin
            .strip_prefix("file:")
            .map(|path| input_path(repository, path));
        if let Some(origin) = &origin {
            paths.insert(origin.clone());
        }
        if key.starts_with("includeif.") || key == "extensions.worktreeconfig" {
            conditional = true;
        }
        if key == "include.path" || (key.starts_with("includeif.") && key.ends_with(".path")) {
            let path = PathBuf::from(value);
            paths.insert(if path.is_absolute() {
                path
            } else {
                origin?.parent()?.join(path)
            });
        } else if matches!(key, "core.excludesfile" | "core.attributesfile") {
            for context in contexts {
                paths.insert(input_path(context, value));
            }
        }
    }
    Some(conditional)
}

fn git_input_output(repository: &Path, args: &[&str]) -> Option<String> {
    let output = gwt_core::process::run_git_logged(args, Some(repository)).ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

fn input_path(repository: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        repository.join(path)
    }
}

fn file_fingerprint(path: &Path) -> Option<Option<[u8; 32]>> {
    match fs::read(path) {
        Ok(bytes) => Some(Some(Sha256::digest(bytes).into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
    }
}

fn git_input_environment() -> Vec<(OsString, OsString)> {
    let mut values: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| {
            let key = key.to_string_lossy();
            key.starts_with("GIT_")
                || matches!(
                    key.as_ref(),
                    "HOME" | "USERPROFILE" | "XDG_CONFIG_HOME" | "GIT_EXEC_PATH"
                )
        })
        .collect();
    values.sort();
    values
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
        // The command repository may itself be a linked worktree even when
        // no active Work row currently names it. Keep its per-worktree config.
        for (index, path) in worktree_paths
            .iter()
            .map(PathBuf::as_path)
            .chain(std::iter::once(project_root))
            .enumerate()
        {
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
            if index < worktree_paths.len() {
                worktrees.push(worktree);
            }
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
            .observe_roots(&repo, std::slice::from_ref(&repo))
            .expect("watch repository");
        assert_eq!(
            inputs.observe_roots(&repo, std::slice::from_ref(&repo)),
            Some(first)
        );
        assert_eq!(inputs.current_revision(), Some(first));
        assert_eq!(gwt_core::process::thread_git_spawn_count(), before);
    }

    #[test]
    fn external_inputs_notice_creation_and_same_size_edits() {
        let _env_lock = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("global-config");
        let mut inputs = ExternalGitInputs {
            contexts: Vec::new(),
            environment: git_input_environment(),
            files: vec![(path.clone(), file_fingerprint(&path).unwrap())],
        };
        assert!(inputs.is_current());
        fs::write(&path, "first").unwrap();
        assert!(
            !inputs.is_current(),
            "an absent global input may be created later"
        );
        inputs.files[0].1 = file_fingerprint(&path).unwrap();
        assert!(inputs.is_current());
        fs::write(&path, "other").unwrap();
        assert!(
            !inputs.is_current(),
            "same-size content edits must invalidate the scan"
        );
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
                .observe_roots(&linked, std::slice::from_ref(&linked))
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
            .observe_roots(&repo, std::slice::from_ref(&linked))
            .expect("watch roots");
        fs::remove_dir(&linked).unwrap();
        assert_eq!(
            inputs.observe_roots(&repo, std::slice::from_ref(&linked)),
            None
        );
        assert_eq!(inputs.current_revision(), None);
        fs::create_dir(&linked).unwrap();
        let recovered = inputs
            .observe_roots(&repo, std::slice::from_ref(&linked))
            .expect("watch recreated tree");
        assert_ne!(recovered, first);
        assert_eq!(inputs.observe_roots(&repo, &[linked]), Some(recovered));
        assert_eq!(
            inputs.observe_roots(&temp.path().join("missing"), &[]),
            None
        );
    }
}
