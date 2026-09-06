//! Tokio job spawning and lifecycle reconciliation for the index runner.
//!
//! This module owns the Rust side of:
//! - Reconciling `~/.gwt/index/<repo-hash>/worktrees/` against `git worktree list`
//!   and removing orphans + legacy `$WORKTREE/.gwt/index/` directories
//! - Cleaning up legacy worktree-scoped SPEC index artifacts after SPEC index
//!   moved to the repo root
//! - Refreshing the Issue index according to a TTL window
//! - Spawning the Python runner as background tokio tasks
//!
//! The actual ChromaDB writes happen inside the Python runner. This module
//! never touches sqlite directly.

use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::{
    error::{GwtError, Result},
    index::view::{FileIndexGcPinDescriptor, WorktreeViewDescriptor, WorktreeViewHead},
    repo_hash::RepoHash,
    worktree_hash::compute_worktree_hash,
};

pub use crate::index::view::FileIndexGcPinKind;

// =====================================================================
// reconcile_repo
// =====================================================================

/// Inputs needed to reconcile the index storage layout for a single repo.
#[derive(Debug, Clone)]
pub struct ReconcileOptions {
    /// Override of `~/.gwt/index/`. Tests inject a tempdir here.
    pub index_root: PathBuf,
    pub repo_hash: RepoHash,
    /// Absolute paths of every worktree currently registered with git for
    /// this repository.
    pub active_worktree_paths: Vec<PathBuf>,
    /// Absolute paths of every worktree where a legacy `$WORKTREE/.gwt/index/`
    /// directory should be removed.
    pub legacy_worktree_dirs: Vec<PathBuf>,
}

/// Reconcile orphan worktree-hash directories under
/// `<index_root>/<repo>/worktrees/` and delete legacy `$WORKTREE/.gwt/index/`.
pub fn reconcile_repo(opts: &ReconcileOptions) -> Result<()> {
    // 1. Compute the set of valid wt-hashes from the active worktree paths.
    let mut valid_hashes = std::collections::HashSet::new();
    for path in &opts.active_worktree_paths {
        if let Ok(h) = compute_worktree_hash(path) {
            let hash = h.as_str().to_string();
            remove_legacy_worktree_specs_artifacts(&opts.index_root, &opts.repo_hash, &hash)?;
            valid_hashes.insert(hash);
        }
    }

    // 2. Walk <index_root>/<repo>/worktrees/ and remove orphans.
    let worktrees_dir = opts
        .index_root
        .join(opts.repo_hash.as_str())
        .join("worktrees");
    if worktrees_dir.is_dir() {
        for entry in std::fs::read_dir(&worktrees_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let hash = name.to_string_lossy().to_string();
            if !valid_hashes.contains(&hash) {
                let path = entry.path();
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                }
            }
        }
    }

    // 3. Remove legacy $WORKTREE/.gwt/index/ directories.
    for wt in &opts.legacy_worktree_dirs {
        let legacy = wt.join(".gwt").join("index");
        if legacy.exists() {
            let _ = std::fs::remove_dir_all(&legacy);
        }
    }

    Ok(())
}

fn remove_legacy_worktree_specs_artifacts(
    index_root: &Path,
    repo: &RepoHash,
    worktree_hash: &str,
) -> Result<()> {
    let worktree_dir = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(worktree_hash);
    let legacy_specs = worktree_dir.join("specs");
    if legacy_specs.exists() {
        std::fs::remove_dir_all(&legacy_specs)?;
    }
    let legacy_manifest = worktree_dir.join("manifest-specs.json");
    if legacy_manifest.exists() {
        std::fs::remove_file(&legacy_manifest)?;
    }
    Ok(())
}

/// Synchronously remove the index directory for a single worktree.
///
/// Called by non-interactive worktree lifecycle handlers when a worktree is
/// removed and its per-worktree file indexes should be deleted eagerly.
pub fn remove_worktree_index(
    index_root: &Path,
    repo: &RepoHash,
    worktree_hash: &str,
) -> Result<()> {
    let _ = repo;
    let target = index_root
        .join(repo.as_str())
        .join("worktrees")
        .join(worktree_hash);
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    Ok(())
}

// =====================================================================
// Phase 71 reachability GC
// =====================================================================

const FILE_INDEX_V2_DIR: &str = "file-index-v2";
const GC_LEASES_DIR: &str = "leases";
const GC_LOCK_FILE: &str = ".lock";
const GC_PIN_FILE: &str = "pin.json";
const GC_ORPHAN_MARKER: &str = ".gc-orphaned.json";

/// Deterministic inputs for one repository-scoped file-index v2 sweep.
#[derive(Debug, Clone)]
pub struct FileIndexGcOptions {
    pub index_root: PathBuf,
    pub repo_hash: RepoHash,
    pub active_worktree_hashes: Vec<String>,
    pub now_unix_nanos: u64,
    pub artifact_ttl: Duration,
    pub worktree_grace: Duration,
}

/// Observable outcome of a best-effort sweep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileIndexGcReport {
    pub deleted: Vec<PathBuf>,
    pub retry_pending: Vec<PathBuf>,
}

/// A cross-process liveness pin held while a reader, migration, or
/// continuation needs immutable v2 artifacts.
///
/// `pin.json` is diagnostic/mark metadata. The sibling kernel-locked `.lock`
/// file is the liveness source of truth, so process exit releases the pin even
/// when its directory remains behind.
pub struct FileIndexGcPin {
    lock_file: Option<File>,
    pin_dir: PathBuf,
    setup_lock_path: PathBuf,
}

impl FileIndexGcPin {
    pub fn acquire(
        v2_root: &Path,
        kind: FileIndexGcPinKind,
        repo_hash: &str,
        worktree_hash: Option<&str>,
        protected_paths: Vec<PathBuf>,
    ) -> Result<Self> {
        let leases_root = v2_root.join(GC_LEASES_DIR);
        fs::create_dir_all(&leases_root)?;
        let setup_lock_path = leases_root.join(GC_LOCK_FILE);
        let setup_lock = open_gc_lock(&setup_lock_path)?;
        FileExt::lock_exclusive(&setup_lock).map_err(|error| {
            GwtError::Other(format!(
                "lock file-index GC lease registry {}: {error}",
                setup_lock_path.display()
            ))
        })?;

        let mut created_pin_dir = None;
        let result = (|| {
            let pin_id = uuid::Uuid::new_v4().to_string();
            let pin_dir = leases_root.join(&pin_id);
            fs::create_dir(&pin_dir)?;
            created_pin_dir = Some(pin_dir.clone());
            let lock_path = pin_dir.join(GC_LOCK_FILE);
            let lock_file = open_gc_lock(&lock_path)?;
            FileExt::lock_shared(&lock_file).map_err(|error| {
                GwtError::Other(format!(
                    "lock file-index GC pin {}: {error}",
                    lock_path.display()
                ))
            })?;

            let protected_paths = protected_paths
                .iter()
                .map(|path| relative_gc_path_to_wire(path))
                .collect::<Result<Vec<_>>>()?;
            let marker = FileIndexGcPinDescriptor::new(
                pin_id,
                kind,
                repo_hash.to_string(),
                worktree_hash.map(str::to_string),
                protected_paths,
                std::process::id(),
                Utc::now().to_rfc3339(),
            )
            .map_err(|error| GwtError::Other(format!("create file-index GC pin: {error}")))?;
            write_json_atomic(&pin_dir.join(GC_PIN_FILE), &marker)?;
            Ok(Self {
                lock_file: Some(lock_file),
                pin_dir,
                setup_lock_path: setup_lock_path.clone(),
            })
        })();

        if result.is_err() {
            if let Some(pin_dir) = created_pin_dir {
                let _ = fs::remove_dir_all(pin_dir);
            }
        }
        let _ = FileExt::unlock(&setup_lock);
        result
    }
}

impl Drop for FileIndexGcPin {
    fn drop(&mut self) {
        let setup_lock = open_gc_lock(&self.setup_lock_path).ok();
        let setup_locked = setup_lock
            .as_ref()
            .is_some_and(|file| FileExt::lock_exclusive(file).is_ok());
        if let Some(lock_file) = self.lock_file.take() {
            let _ = FileExt::unlock(&lock_file);
            drop(lock_file);
        }
        if setup_locked {
            let _ = fs::remove_dir_all(&self.pin_dir);
        }
        if let Some(setup_lock) = setup_lock {
            let _ = FileExt::unlock(&setup_lock);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorktreeGcGraceMarker {
    schema_version: u32,
    first_absent_unix_nanos: u64,
}

pub fn sweep_file_index_v2(options: &FileIndexGcOptions) -> Result<FileIndexGcReport> {
    sweep_file_index_v2_with_remover(options, fs::remove_dir_all)
}

pub fn sweep_file_index_v2_with_remover<F>(
    options: &FileIndexGcOptions,
    remover: F,
) -> Result<FileIndexGcReport>
where
    F: FnMut(PathBuf) -> io::Result<()>,
{
    sweep_file_index_v2_inner(options, remover)
}

fn sweep_file_index_v2_inner<F>(
    options: &FileIndexGcOptions,
    mut remover: F,
) -> Result<FileIndexGcReport>
where
    F: FnMut(PathBuf) -> io::Result<()>,
{
    let v2_root = options
        .index_root
        .join(options.repo_hash.as_str())
        .join(FILE_INDEX_V2_DIR);
    if !v2_root.is_dir() {
        return Ok(FileIndexGcReport::default());
    }

    let leases_root = v2_root.join(GC_LEASES_DIR);
    fs::create_dir_all(&leases_root)?;
    let setup_lock_path = leases_root.join(GC_LOCK_FILE);
    let setup_lock = open_gc_lock(&setup_lock_path)?;
    FileExt::lock_exclusive(&setup_lock).map_err(|error| {
        GwtError::Other(format!(
            "lock file-index GC lease registry {}: {error}",
            setup_lock_path.display()
        ))
    })?;

    let sweep_result = (|| {
        let mut protected = HashSet::new();
        let mut candidates = Vec::new();
        collect_gc_pin_roots(
            &v2_root,
            options.repo_hash.as_str(),
            &mut protected,
            &mut candidates,
        )?;
        collect_worktree_roots(options, &v2_root, &mut protected, &mut candidates)?;
        collect_expired_gc_artifacts(
            &v2_root,
            options.now_unix_nanos,
            options.artifact_ttl,
            &mut candidates,
        )?;

        candidates.sort_by(|left, right| {
            left.components()
                .count()
                .cmp(&right.components().count())
                .then_with(|| left.cmp(right))
        });
        candidates.dedup();

        let mut report = FileIndexGcReport::default();
        for candidate in candidates {
            if !candidate.exists() || gc_paths_intersect(&candidate, &protected) {
                continue;
            }
            match remover(candidate.clone()) {
                Ok(()) => report.deleted.push(candidate),
                Err(_) => report.retry_pending.push(candidate),
            }
        }
        Ok(report)
    })();

    let _ = FileExt::unlock(&setup_lock);
    sweep_result
}

fn open_gc_lock(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
}

fn relative_gc_path_to_wire(path: &Path) -> Result<String> {
    if path.is_absolute() {
        return Err(GwtError::Other(format!(
            "file-index GC pin path must be relative: {}",
            path.display()
        )));
    }
    let components = path
        .components()
        .map(|component| match component {
            std::path::Component::Normal(value) => value
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| GwtError::Other("file-index GC pin path must be UTF-8".to_string())),
            _ => Err(GwtError::Other(format!(
                "file-index GC pin path is unsafe: {}",
                path.display()
            ))),
        })
        .collect::<Result<Vec<_>>>()?;
    if components.is_empty() {
        return Err(GwtError::Other(
            "file-index GC pin path must not be empty".to_string(),
        ));
    }
    Ok(components.join("/"))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        GwtError::Other(format!(
            "file-index GC path has no parent: {}",
            path.display()
        ))
    })?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("gc-json"),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let bytes = serde_json::to_vec(value)
            .map_err(|error| GwtError::Other(format!("serialize file-index GC JSON: {error}")))?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        sync_gc_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn sync_gc_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_gc_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn is_gc_lock_contended(error: &io::Error) -> bool {
    crate::operation_deadline::is_lock_contended(error)
}

fn collect_gc_pin_roots(
    v2_root: &Path,
    expected_repo_hash: &str,
    protected: &mut HashSet<PathBuf>,
    candidates: &mut Vec<PathBuf>,
) -> Result<()> {
    let leases_root = v2_root.join(GC_LEASES_DIR);
    let mut entries = fs::read_dir(&leases_root)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if entry.file_name() == GC_LOCK_FILE || !entry.file_type()?.is_dir() {
            continue;
        }
        let pin_dir = entry.path();
        let lock_path = pin_dir.join(GC_LOCK_FILE);
        let lock_file = open_gc_lock(&lock_path)?;
        match FileExt::try_lock_exclusive(&lock_file) {
            Ok(()) => {
                let _ = FileExt::unlock(&lock_file);
                drop(lock_file);
                candidates.push(pin_dir);
            }
            Err(error) if is_gc_lock_contended(&error) => {
                let marker_path = pin_dir.join(GC_PIN_FILE);
                let marker_bytes = fs::read(&marker_path).map_err(|error| {
                    GwtError::Other(format!(
                        "read live file-index GC pin {}: {error}",
                        marker_path.display()
                    ))
                })?;
                let marker: FileIndexGcPinDescriptor = serde_json::from_slice(&marker_bytes)
                    .map_err(|error| {
                        GwtError::Other(format!(
                            "invalid live file-index GC pin {}: {error}",
                            marker_path.display()
                        ))
                    })?;
                let directory_pin_id = entry.file_name().to_string_lossy().into_owned();
                if marker.pin_id != directory_pin_id || marker.repo_hash != expected_repo_hash {
                    return Err(GwtError::Other(format!(
                        "file-index GC pin authority mismatch at {}",
                        marker_path.display()
                    )));
                }
                for relative in &marker.protected_paths {
                    protected.insert(v2_root.join(relative));
                }
            }
            Err(error) => {
                return Err(GwtError::Other(format!(
                    "probe file-index GC pin {}: {error}",
                    lock_path.display()
                )));
            }
        }
    }
    Ok(())
}

fn collect_worktree_roots(
    options: &FileIndexGcOptions,
    v2_root: &Path,
    protected: &mut HashSet<PathBuf>,
    candidates: &mut Vec<PathBuf>,
) -> Result<()> {
    let worktrees_root = v2_root.join("worktrees");
    if !worktrees_root.is_dir() {
        return Ok(());
    }
    let active = options
        .active_worktree_hashes
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut entries = fs::read_dir(&worktrees_root)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let worktree_root = entry.path();
        let worktree_hash = entry.file_name().to_string_lossy().to_string();
        let grace_marker_path = worktree_root.join(GC_ORPHAN_MARKER);
        let retain_head_closure = if active.contains(worktree_hash.as_str()) {
            if grace_marker_path.exists() {
                fs::remove_file(&grace_marker_path)?;
            }
            true
        } else {
            let marker = if grace_marker_path.exists() {
                let bytes = fs::read(&grace_marker_path)?;
                let marker: WorktreeGcGraceMarker =
                    serde_json::from_slice(&bytes).map_err(|error| {
                        GwtError::Other(format!(
                            "invalid file-index worktree grace marker {}: {error}",
                            grace_marker_path.display()
                        ))
                    })?;
                if marker.schema_version != 1 {
                    return Err(GwtError::Other(format!(
                        "unsupported file-index worktree grace marker {}",
                        grace_marker_path.display()
                    )));
                }
                marker
            } else {
                let marker = WorktreeGcGraceMarker {
                    schema_version: 1,
                    first_absent_unix_nanos: options.now_unix_nanos,
                };
                write_json_atomic(&grace_marker_path, &marker)?;
                marker
            };
            let elapsed = options
                .now_unix_nanos
                .saturating_sub(marker.first_absent_unix_nanos) as u128;
            if elapsed > options.worktree_grace.as_nanos() {
                candidates.push(worktree_root.clone());
                false
            } else {
                true
            }
        };

        if retain_head_closure {
            collect_head_file_closure(
                v2_root,
                options.repo_hash.as_str(),
                &worktree_hash,
                &worktree_root.join("head.json"),
                protected,
            )?;
            collect_head_file_closure(
                v2_root,
                options.repo_hash.as_str(),
                &worktree_hash,
                &worktree_root.join("head.previous.json"),
                protected,
            )?;
        }
    }
    Ok(())
}

fn collect_head_file_closure(
    v2_root: &Path,
    expected_repo_hash: &str,
    expected_worktree_hash: &str,
    head_path: &Path,
    protected: &mut HashSet<PathBuf>,
) -> Result<()> {
    if !head_path.is_file() {
        return Ok(());
    }
    let head_bytes = fs::read(head_path)?;
    let head: WorktreeViewHead = serde_json::from_slice(&head_bytes).map_err(|error| {
        GwtError::Other(format!(
            "invalid file-index WorktreeView head {}: {error}",
            head_path.display()
        ))
    })?;
    let mut view_ids = vec![head.active_view_id];
    if let Some(previous) = head.previous_view_id {
        view_ids.push(previous);
    }
    for view_id in view_ids {
        let view_dir = v2_root
            .join("worktrees")
            .join(expected_worktree_hash)
            .join("views")
            .join(&view_id);
        let descriptor_path = view_dir.join("descriptor.json");
        let descriptor_bytes = fs::read(&descriptor_path).map_err(|error| {
            GwtError::Other(format!(
                "read file-index WorktreeView descriptor {}: {error}",
                descriptor_path.display()
            ))
        })?;
        let descriptor: WorktreeViewDescriptor = serde_json::from_slice(&descriptor_bytes)
            .map_err(|error| {
                GwtError::Other(format!(
                    "invalid file-index WorktreeView descriptor {}: {error}",
                    descriptor_path.display()
                ))
            })?;
        if descriptor.view_id != view_id
            || descriptor.repo_hash != expected_repo_hash
            || descriptor.worktree_hash != expected_worktree_hash
        {
            return Err(GwtError::Other(format!(
                "file-index WorktreeView authority mismatch at {}",
                descriptor_path.display()
            )));
        }
        protected.insert(view_dir);
        protected.insert(v2_root.join("bases").join(descriptor.base_generation_id));
        protected.insert(
            v2_root
                .join("worktrees")
                .join(expected_worktree_hash)
                .join("overlays")
                .join(descriptor.overlay_generation_id),
        );
    }
    Ok(())
}

fn collect_expired_gc_artifacts(
    root: &Path,
    now_unix_nanos: u64,
    ttl: Duration,
    candidates: &mut Vec<PathBuf>,
) -> Result<()> {
    let mut entries = fs::read_dir(root)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(created_at) = gc_artifact_created_at(&name) {
            let elapsed = now_unix_nanos.saturating_sub(created_at) as u128;
            if elapsed > ttl.as_nanos() {
                candidates.push(path);
                continue;
            }
        }
        collect_expired_gc_artifacts(&path, now_unix_nanos, ttl, candidates)?;
    }
    Ok(())
}

fn gc_artifact_created_at(name: &str) -> Option<u64> {
    if !name.starts_with('.') {
        return None;
    }
    let (prefix_and_time, pid) = name.rsplit_once('-')?;
    pid.parse::<u32>().ok()?;
    let (prefix, created_at) = prefix_and_time.rsplit_once('-')?;
    if !prefix.ends_with(".staging") && !prefix.ends_with(".quarantine") {
        return None;
    }
    created_at.parse().ok()
}

fn gc_paths_intersect(candidate: &Path, protected: &HashSet<PathBuf>) -> bool {
    protected
        .iter()
        .any(|root| candidate.starts_with(root) || root.starts_with(candidate))
}

// =====================================================================
// refresh_issues_if_stale
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IssueMetadata {
    schema_version: u32,
    last_full_refresh: String,
    ttl_minutes: u64,
}

/// Trait abstraction over the Python runner spawn so tests can substitute a
/// recording double.
pub trait RunnerSpawner: Send + Sync {
    fn spawn_index_issues(
        &self,
        repo_hash: &str,
        project_root: &Path,
        respect_ttl: bool,
    ) -> std::io::Result<()>;
}

#[derive(Debug, Clone)]
pub struct RefreshIssuesOptions {
    pub index_root: PathBuf,
    pub repo_hash: RepoHash,
    pub project_root: PathBuf,
    pub ttl: Duration,
}

/// Outcome of a single `refresh_issues_if_stale` invocation. Lets callers
/// distinguish "actually spawned a runner" from "TTL still valid, skipped".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshDecision {
    /// Index was missing or stale; the spawner was invoked.
    Spawned,
    /// TTL has not expired yet. `remaining_seconds` is how long until the
    /// next refresh becomes due.
    SkippedWithinTtl { remaining_seconds: u64 },
}

/// Refresh the Issue index if (a) no metadata exists, or (b) the recorded
/// `last_full_refresh` is older than `ttl`. Returns immediately after
/// dispatching to the spawner — the spawner is responsible for any
/// background work.
pub async fn refresh_issues_if_stale<S: RunnerSpawner + ?Sized>(
    opts: &RefreshIssuesOptions,
    spawner: &S,
) -> Result<RefreshDecision> {
    let issues_dir = gwt_index_repo_dir_under(&opts.index_root, &opts.repo_hash).join("issues");
    let meta_path = issues_dir.join("meta.json");
    let mut remaining_seconds: u64 = 0;
    let stale = if meta_path.is_file() {
        match read_issue_meta(&meta_path) {
            Some(meta) => match DateTime::parse_from_rfc3339(&meta.last_full_refresh) {
                Ok(dt) => {
                    let age = Utc::now().signed_duration_since(dt.with_timezone(&Utc));
                    let age_std = age.to_std().unwrap_or(Duration::MAX);
                    if age_std >= opts.ttl {
                        true
                    } else {
                        remaining_seconds = (opts.ttl - age_std).as_secs();
                        false
                    }
                }
                Err(_) => true,
            },
            None => true,
        }
    } else {
        true
    };

    if stale {
        spawner
            .spawn_index_issues(opts.repo_hash.as_str(), &opts.project_root, false)
            .map_err(|e| GwtError::Other(format!("spawn issue index: {e}")))?;
        Ok(RefreshDecision::Spawned)
    } else {
        Ok(RefreshDecision::SkippedWithinTtl { remaining_seconds })
    }
}

fn gwt_index_repo_dir_under(index_root: &Path, repo: &RepoHash) -> PathBuf {
    index_root.join(repo.as_str())
}

fn read_issue_meta(path: &Path) -> Option<IssueMetadata> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// FR-394 post-lock revalidation: true when the issue index meta records a
/// full refresh at or after `since`, meaning an equivalent refresh finished
/// while this caller waited for the coordinator lock and the duplicate can
/// be skipped.
pub fn issue_index_refreshed_since(
    index_root: &Path,
    repo_hash: &str,
    since: DateTime<Utc>,
) -> bool {
    let meta_path = index_root.join(repo_hash).join("issues").join("meta.json");
    let Some(meta) = read_issue_meta(&meta_path) else {
        return false;
    };
    let Ok(last) = DateTime::parse_from_rfc3339(&meta.last_full_refresh) else {
        return false;
    };
    last.with_timezone(&Utc) >= since
}

// =====================================================================
// Default RunnerSpawner: spawns the actual Python runner via tokio
// =====================================================================

/// Default `RunnerSpawner` that fires the real Python runner in a detached
/// tokio task. Used by the desktop app in production; tests prefer a recording
/// double.
#[derive(Debug, Clone)]
pub struct PythonRunnerSpawner {
    pub python_executable: PathBuf,
    pub runner_script: PathBuf,
}

impl PythonRunnerSpawner {
    /// Spawn the detached issue index runner with an explicit coordinator
    /// root. Tests use this seam to avoid sharing host-wide coordinator state.
    pub fn spawn_index_issues_with_coordinator_root(
        &self,
        repo_hash: &str,
        project_root: &Path,
        respect_ttl: bool,
        coordinator_root: &Path,
    ) -> std::io::Result<()> {
        self.spawn_index_issues_with_coordinator(
            repo_hash,
            project_root,
            respect_ttl,
            Some(coordinator_root.to_path_buf()),
        )
    }

    fn spawn_index_issues_with_coordinator(
        &self,
        repo_hash: &str,
        project_root: &Path,
        respect_ttl: bool,
        coordinator_root: Option<PathBuf>,
    ) -> std::io::Result<()> {
        // A missing venv python must surface synchronously to the caller;
        // once the job is detached behind the coordinator only logs would
        // see it.
        if !self.python_executable.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "index runner python not found: {}",
                    self.python_executable.display()
                ),
            ));
        }
        // SPEC-1924 FR-039 / SPEC-2809 Phase D-runner — emit a
        // `gwt.process.summary` start event so the Console window's
        // runner tab and the Logs Process facet observe the Python
        // chroma index runner spawn.
        let spawn_id = RUNNER_SPAWN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let label = format!(
            "{} {} --action index-issues",
            self.python_executable.display(),
            self.runner_script.display(),
        );
        tracing::info!(
            target: "gwt.process.summary",
            kind = "runner",
            spawn_id = spawn_id,
            label = %label,
            phase = "start",
            respect_ttl = respect_ttl,
            "process start",
        );
        crate::process::push_command_banner_to_hub(
            crate::process_console::ProcessKind::IndexRunner,
            spawn_id,
            &label,
            None,
        );

        let mut cmd = crate::process::hidden_command(&self.python_executable);
        cmd.arg(&self.runner_script)
            .arg("--action")
            .arg("index-issues")
            .arg("--repo-hash")
            .arg(repo_hash)
            .arg("--project-root")
            .arg(project_root)
            .arg("--qos")
            .arg("background");
        if respect_ttl {
            cmd.arg("--respect-ttl");
        }
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // Fire-and-forget for the caller, but the detached thread routes the
        // heavy index build through the host-wide coordinator (SPEC #1939
        // Phase 70 FR-379/FR-382) and drains the child while it runs.
        let repo_hash = repo_hash.to_string();
        std::thread::Builder::new()
            .name("gwt-index-issues".to_string())
            .spawn(move || {
                run_coordinated_issue_index(
                    &repo_hash,
                    cmd,
                    spawn_id,
                    &label,
                    coordinator_root.as_deref(),
                );
            })
            .map(|_| ())
    }
}

impl RunnerSpawner for PythonRunnerSpawner {
    fn spawn_index_issues(
        &self,
        repo_hash: &str,
        project_root: &Path,
        respect_ttl: bool,
    ) -> std::io::Result<()> {
        self.spawn_index_issues_with_coordinator(repo_hash, project_root, respect_ttl, None)
    }
}

/// Timeouts for the coordinated background issue index build.
const ISSUE_INDEX_ADMISSION_TIMEOUT: Duration = Duration::from_secs(30);
const ISSUE_INDEX_HEAVY_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const ISSUE_INDEX_SHARED_WAIT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

fn run_coordinated_issue_index(
    repo_hash: &str,
    mut cmd: std::process::Command,
    spawn_id: u64,
    label: &str,
    coordinator_root: Option<&Path>,
) {
    use crate::index_coordinator::{
        IndexCoordinator, JobAdmission, JobOutcome, JobPriority, TargetKey,
    };

    let coordinator = match coordinator_root
        .map(IndexCoordinator::open)
        .unwrap_or_else(IndexCoordinator::open_default)
    {
        Ok(coordinator) => coordinator,
        Err(err) => {
            tracing::warn!(
                target: "gwt::index",
                spawn_id = spawn_id,
                error = %err,
                "issue index skipped: coordinator unavailable"
            );
            emit_issue_runner_end(spawn_id, label, false);
            return;
        }
    };
    let key = TargetKey::repo_shared(repo_hash, "issues");
    let requested_at = Utc::now();
    match coordinator.request_job(&key, JobPriority::Background, ISSUE_INDEX_ADMISSION_TIMEOUT) {
        Ok(JobAdmission::Owner(guard)) => {
            // FR-394 post-lock revalidation: skip the duplicate when an
            // equivalent refresh completed while we queued for the target.
            if issue_index_refreshed_since(
                &crate::index::paths::gwt_index_root(),
                repo_hash,
                requested_at,
            ) {
                let _ = guard.complete(crate::index_coordinator::JobOutcome::Completed);
                tracing::info!(
                    target: "gwt::index",
                    spawn_id = spawn_id,
                    "issue index skipped: refreshed while waiting for the coordinator"
                );
                emit_issue_runner_end(spawn_id, label, true);
                return;
            }
            let heavy = match guard.acquire_heavy(ISSUE_INDEX_HEAVY_TIMEOUT) {
                Ok(heavy) => heavy,
                Err(err) => {
                    tracing::warn!(
                        target: "gwt::index",
                        spawn_id = spawn_id,
                        error = %err,
                        "issue index skipped: heavy lease unavailable"
                    );
                    let _ = guard.complete(JobOutcome::Failed {
                        message: format!("heavy lease unavailable: {err}"),
                    });
                    emit_issue_runner_end(spawn_id, label, false);
                    return;
                }
            };
            let outcome = match cmd.spawn().and_then(|child| child.wait_with_output()) {
                Ok(output) if output.status.success() => JobOutcome::Completed,
                Ok(output) => {
                    tracing::warn!(
                        target: "gwt::index",
                        spawn_id = spawn_id,
                        exit_status = %output.status,
                        stderr = %String::from_utf8_lossy(&output.stderr),
                        "issue index runner failed"
                    );
                    JobOutcome::Failed {
                        message: format!("issue index runner exited with {}", output.status),
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        target: "gwt::index",
                        spawn_id = spawn_id,
                        error = %err,
                        "issue index runner spawn failed"
                    );
                    JobOutcome::Failed {
                        message: err.to_string(),
                    }
                }
            };
            drop(heavy);
            let completed = matches!(outcome, JobOutcome::Completed);
            let _ = guard.complete(outcome);
            tracing::info!(
                target: "gwt.process.summary",
                kind = "runner",
                spawn_id = spawn_id,
                label = %label,
                phase = "end",
                success = completed,
                "process end",
            );
        }
        Ok(JobAdmission::Joined(waiter)) => {
            // An equivalent issue index build is already running host-wide;
            // coalesce instead of spawning a duplicate model load (FR-382).
            match waiter.wait(ISSUE_INDEX_SHARED_WAIT_TIMEOUT) {
                Ok(crate::index_coordinator::JobOutcome::Completed) => {
                    tracing::info!(
                        target: "gwt::index",
                        spawn_id = spawn_id,
                        "issue index coalesced into a concurrent equivalent job"
                    );
                    emit_issue_runner_end(spawn_id, label, true);
                }
                Ok(outcome) => {
                    tracing::warn!(
                        target: "gwt::index",
                        spawn_id = spawn_id,
                        outcome = ?outcome,
                        "coalesced issue index job did not complete successfully"
                    );
                    emit_issue_runner_end(spawn_id, label, false);
                }
                Err(err) => {
                    tracing::warn!(
                        target: "gwt::index",
                        spawn_id = spawn_id,
                        error = %err,
                        "waiting on the shared issue index job failed"
                    );
                    emit_issue_runner_end(spawn_id, label, false);
                }
            }
        }
        Err(err) => {
            tracing::warn!(
                target: "gwt::index",
                spawn_id = spawn_id,
                error = %err,
                "issue index skipped: job admission failed"
            );
            emit_issue_runner_end(spawn_id, label, false);
        }
    }
}

/// Close the `gwt.process.summary` pair opened at spawn time so the Console
/// runner tab never shows an orphaned start entry (PR #3301 review).
fn emit_issue_runner_end(spawn_id: u64, label: &str, success: bool) {
    tracing::info!(
        target: "gwt.process.summary",
        kind = "runner",
        spawn_id = spawn_id,
        label = %label,
        phase = "end",
        success = success,
        "process end",
    );
}

static RUNNER_SPAWN_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::repo_hash::compute_repo_hash;

    #[test]
    fn gc_recognizes_the_platform_lock_contention_error() {
        assert!(is_gc_lock_contended(&fs2::lock_contended_error()));
    }

    #[derive(Default, Clone)]
    struct RecordingSpawner {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl RunnerSpawner for RecordingSpawner {
        fn spawn_index_issues(
            &self,
            repo_hash: &str,
            project_root: &Path,
            respect_ttl: bool,
        ) -> std::io::Result<()> {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(format!(
                    "{}|{}|{}",
                    repo_hash,
                    project_root.display(),
                    respect_ttl
                ));
            Ok(())
        }
    }

    #[tokio::test]
    async fn refresh_kicks_when_meta_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = compute_repo_hash("https://github.com/example/repo.git");
        let spawner = RecordingSpawner::default();
        let opts = RefreshIssuesOptions {
            index_root: tmp.path().join("idx"),
            repo_hash: repo,
            project_root: tmp.path().to_path_buf(),
            ttl: Duration::from_secs(15 * 60),
        };
        refresh_issues_if_stale(&opts, &spawner).await.unwrap();
        assert_eq!(
            spawner
                .calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            1
        );
    }

    #[test]
    fn reconcile_removes_orphan_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let idx = tmp.path().join("idx");
        let repo = compute_repo_hash("https://github.com/example/repo.git");
        let orphan = idx
            .join(repo.as_str())
            .join("worktrees")
            .join("deadbeefdeadbeef");
        std::fs::create_dir_all(&orphan).unwrap();

        let opts = ReconcileOptions {
            index_root: idx,
            repo_hash: repo,
            active_worktree_paths: Vec::new(),
            legacy_worktree_dirs: Vec::new(),
        };
        reconcile_repo(&opts).unwrap();
        assert!(!orphan.exists());
    }
}
