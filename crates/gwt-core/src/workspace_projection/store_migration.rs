//! Issue #3466 / SPEC-3431 T-173: consolidate project stores that a pre-#3466
//! build split apart.
//!
//! Before the project scope followed the repository identity, every view of a
//! repository that could not resolve `origin` — most importantly the Nested
//! Bare + Worktree layout root, which has no `.git` of its own — was keyed by
//! the hash of its *path*. One repository therefore ended up with several
//! `~/.gwt/projects/<hash>` trees that never saw each other's Work.
//!
//! Resolving the identity (see [`crate::repo_hash::detect_repo_identity`])
//! stops new splits, but the orphaned stores are still on disk. This module
//! folds them back in without ever writing to them:
//!
//! 1. [`plan_store_consolidation`] is a dry run. It names every orphaned store
//!    with its source path, hash, counts and revision, and derives a manifest
//!    hash over that set.
//! 2. [`apply_store_consolidation`] takes an exclusive writer lease, refuses
//!    unless the manifest hash still matches, *moves* each orphan under
//!    `quarantine/` beside a read-only manifest, and rebuilds the canonical
//!    projection from the durable event logs. It reads the result back and
//!    rolls everything back if that fails.
//!
//! Every refusal is [`NeedsHuman`]: an unresolvable remote, a concurrent
//! writer, a plan that no longer matches the disk, corrupt input, or a failed
//! readback all stop the migration rather than guess.

use std::{
    collections::HashSet,
    fmt, fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    paths::{
        gwt_project_dir, gwt_project_state_work_events_path, gwt_project_state_works_path,
        gwt_workspace_work_events_closed_path,
    },
    repo_hash::{
        compute_path_hash, detect_repo_identity, linked_worktree_roots,
        resolve_repository_common_dir, RepoHash,
    },
    work_events_intake::rebuild_work_events_contents,
    workspace_projection::load_workspace_work_items_from_path,
};

/// Schema version of [`QuarantineManifest`].
pub const QUARANTINE_MANIFEST_VERSION: u32 = 1;

const QUARANTINE_DIR: &str = "quarantine";
const LEASE_FILE: &str = "store-consolidation.lock";

/// Why a consolidation stopped. Every variant needs a human decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreConsolidationRefusal {
    /// No repository identity could be resolved, so there is no canonical
    /// store to consolidate into.
    UnknownRemote,
    /// Another process holds the consolidation lease.
    WriterBusy,
    /// The disk no longer matches the plan the caller approved.
    ManifestChanged,
    /// A store could not be read or parsed.
    CorruptInput,
    /// The rebuilt projection did not read back intact; changes were rolled
    /// back.
    ReadbackFailed,
}

impl StoreConsolidationRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownRemote => "unknown_remote",
            Self::WriterBusy => "writer_busy",
            Self::ManifestChanged => "manifest_changed",
            Self::CorruptInput => "corrupt_input",
            Self::ReadbackFailed => "readback_failed",
        }
    }
}

/// A fail-closed stop. The migration made no net change when this is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsHuman {
    pub refusal: StoreConsolidationRefusal,
    pub detail: String,
}

impl NeedsHuman {
    fn new(refusal: StoreConsolidationRefusal, detail: impl Into<String>) -> Self {
        Self {
            refusal,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for NeedsHuman {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NeedsHuman({}): {}", self.refusal.as_str(), self.detail)
    }
}

impl std::error::Error for NeedsHuman {}

pub type ConsolidationResult<T> = std::result::Result<T, NeedsHuman>;

/// A project store a pre-#3466 build keyed by path instead of by repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrphanedStore {
    /// The path whose hash keyed the store.
    pub source_root: PathBuf,
    /// That path hash, i.e. the store's directory name.
    pub source_hash: String,
    /// `~/.gwt/projects/<source_hash>`.
    pub store_dir: PathBuf,
    pub work_item_count: usize,
    pub work_event_count: usize,
    /// Content revision of the store's `works.json`, so a manifest identifies
    /// exactly which bytes were quarantined.
    pub revision: String,
}

/// The read-only record left beside a quarantined store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantineManifest {
    pub version: u32,
    pub canonical_hash: String,
    pub quarantined_at: DateTime<Utc>,
    pub source: OrphanedStore,
}

/// The dry-run result: what consolidation would do, and the hash that pins it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreConsolidationPlan {
    pub project_root: PathBuf,
    pub canonical_hash: RepoHash,
    pub canonical_store: PathBuf,
    pub orphans: Vec<OrphanedStore>,
    /// Digest over the canonical hash and every orphan. [`apply_store_consolidation`]
    /// refuses unless the disk still produces this value.
    pub manifest_hash: String,
}

impl StoreConsolidationPlan {
    /// Whether applying this plan would change anything.
    pub fn is_empty(&self) -> bool {
        self.orphans.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreConsolidationOutcome {
    /// No orphaned store remained, so the canonical store was left untouched.
    NothingToDo,
    Consolidated {
        /// The quarantine directory each orphan was moved to.
        quarantined: Vec<PathBuf>,
        /// Work items in the rebuilt canonical projection.
        work_item_count: usize,
    },
}

/// Dry run: report every orphaned store for `project_root` without writing.
pub fn plan_store_consolidation(
    project_root: &Path,
) -> ConsolidationResult<StoreConsolidationPlan> {
    let identity = detect_repo_identity(project_root).ok_or_else(|| {
        NeedsHuman::new(
            StoreConsolidationRefusal::UnknownRemote,
            format!(
                "no origin resolved for {}; consolidation has no canonical target",
                project_root.display()
            ),
        )
    })?;
    let canonical_store = gwt_project_dir(&identity.hash);
    let mut orphans = Vec::new();
    // Several candidate paths can alias one directory (a symlinked or
    // `/private`-prefixed temp root), and they all hash the same. Report each
    // store once so the apply never tries to move it twice.
    let mut seen = HashSet::new();
    for source_root in split_candidate_roots(project_root) {
        let source_hash = compute_path_hash(&source_root);
        if source_hash == identity.hash || !seen.insert(source_hash.as_str().to_string()) {
            continue;
        }
        let store_dir = gwt_project_dir(&source_hash);
        if !store_dir.is_dir() {
            continue;
        }
        // Already quarantined by an earlier run: skip so apply is idempotent.
        if quarantine_manifest_path(&canonical_store, source_hash.as_str()).is_file() {
            continue;
        }
        orphans.push(inspect_store(source_root, &source_hash, store_dir)?);
    }
    orphans.sort_by(|left, right| left.source_hash.cmp(&right.source_hash));
    let manifest_hash = compute_manifest_hash(identity.hash.as_str(), &orphans);
    Ok(StoreConsolidationPlan {
        project_root: project_root.to_path_buf(),
        canonical_hash: identity.hash,
        canonical_store,
        orphans,
        manifest_hash,
    })
}

/// Quarantine every orphaned store and rebuild the canonical projection.
///
/// `expected_manifest_hash` is the [`StoreConsolidationPlan::manifest_hash`]
/// the caller reviewed. Applying the same plan twice is a no-op.
pub fn apply_store_consolidation(
    project_root: &Path,
    expected_manifest_hash: &str,
) -> ConsolidationResult<StoreConsolidationOutcome> {
    let plan = plan_store_consolidation(project_root)?;
    if plan.manifest_hash != expected_manifest_hash {
        return Err(NeedsHuman::new(
            StoreConsolidationRefusal::ManifestChanged,
            format!(
                "plan {} no longer matches the approved manifest {expected_manifest_hash}",
                plan.manifest_hash
            ),
        ));
    }
    if plan.is_empty() {
        return Ok(StoreConsolidationOutcome::NothingToDo);
    }

    let _lease = WriterLease::acquire(&plan.canonical_store)?;

    // Re-plan under the lease: a writer may have moved between the dry run and
    // now, and the lease is what makes this observation stable.
    let confirmed = plan_store_consolidation(project_root)?;
    if confirmed.manifest_hash != expected_manifest_hash {
        return Err(NeedsHuman::new(
            StoreConsolidationRefusal::ManifestChanged,
            format!(
                "store changed while acquiring the lease: {} != {expected_manifest_hash}",
                confirmed.manifest_hash
            ),
        ));
    }

    let canonical_works = gwt_project_state_works_path(&confirmed.canonical_hash);
    let works_snapshot = fs::read(&canonical_works).ok();
    let mut moved: Vec<MovedStore> = Vec::new();

    let result = (|| -> ConsolidationResult<usize> {
        for orphan in &confirmed.orphans {
            moved.push(quarantine_store(&confirmed, orphan)?);
        }
        rebuild_canonical_projection(&confirmed, &canonical_works, &moved)
    })();

    match result {
        Ok(work_item_count) => Ok(StoreConsolidationOutcome::Consolidated {
            quarantined: moved.into_iter().map(|entry| entry.quarantined).collect(),
            work_item_count,
        }),
        Err(error) => {
            rollback(&canonical_works, works_snapshot.as_deref(), &moved);
            Err(error)
        }
    }
}

/// The paths whose hashes could have keyed a split store for this repository:
/// the project root itself, the shared git directory's own parent, and every
/// registered worktree. Bounded on purpose — an unrelated directory must never
/// be swept into a repository's store (#3466 AC-9).
fn split_candidate_roots(project_root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![project_root.to_path_buf()];
    if let Some(common_dir) = resolve_repository_common_dir(project_root) {
        // A linked worktree resolves its common dir through a relative
        // `commondir` (`../..`), so the path still carries `..` components.
        // `Path::parent` only strips the last component, which would name a
        // directory inside the git dir instead of the layout root — canonicalize
        // before stepping up.
        let common_dir = dunce::canonicalize(&common_dir).unwrap_or(common_dir);
        if let Some(parent) = common_dir.parent() {
            roots.push(parent.to_path_buf());
        }
        roots.extend(linked_worktree_roots(&common_dir));
    }
    // Discovery order, not sorted: the caller's own root is reported first when
    // several paths alias the same directory (`/var` vs `/private/var`). The
    // manifest hash keys on the store hash, so the alias chosen never changes
    // which plan a caller approved.
    roots.dedup();
    roots
}

fn inspect_store(
    source_root: PathBuf,
    source_hash: &RepoHash,
    store_dir: PathBuf,
) -> ConsolidationResult<OrphanedStore> {
    let works_path = gwt_project_state_works_path(source_hash);
    let works_bytes = read_optional_bytes(&works_path)?;
    let work_item_count = if works_bytes.is_some() {
        load_workspace_work_items_from_path(&works_path)
            .map_err(|error| {
                NeedsHuman::new(
                    StoreConsolidationRefusal::CorruptInput,
                    format!("{}: {error}", works_path.display()),
                )
            })?
            .ok_or_else(|| {
                NeedsHuman::new(
                    StoreConsolidationRefusal::CorruptInput,
                    format!("{} is present but unreadable", works_path.display()),
                )
            })?
            .work_items
            .len()
    } else {
        0
    };
    let events_path = gwt_project_state_work_events_path(source_hash);
    let work_event_count = read_optional_string(&events_path)?
        .map(|content| {
            content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
        })
        .unwrap_or(0);
    Ok(OrphanedStore {
        source_root,
        source_hash: source_hash.as_str().to_string(),
        store_dir,
        work_item_count,
        work_event_count,
        revision: content_revision(works_bytes.as_deref()),
    })
}

struct MovedStore {
    source: PathBuf,
    quarantined: PathBuf,
    manifest: PathBuf,
}

fn quarantine_store(
    plan: &StoreConsolidationPlan,
    orphan: &OrphanedStore,
) -> ConsolidationResult<MovedStore> {
    let canonical_store = plan.canonical_store.as_path();
    let quarantine_root = canonical_store.join(QUARANTINE_DIR);
    fs::create_dir_all(&quarantine_root).map_err(|error| {
        NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!("{}: {error}", quarantine_root.display()),
        )
    })?;
    let destination = quarantine_root.join(&orphan.source_hash);
    fs::rename(&orphan.store_dir, &destination).map_err(|error| {
        NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!(
                "could not quarantine {}: {error}",
                orphan.store_dir.display()
            ),
        )
    })?;
    let manifest_path = quarantine_manifest_path(canonical_store, &orphan.source_hash);
    let manifest = QuarantineManifest {
        version: QUARANTINE_MANIFEST_VERSION,
        canonical_hash: plan.canonical_hash.as_str().to_string(),
        quarantined_at: Utc::now(),
        source: orphan.clone(),
    };
    let encoded = serde_json::to_vec_pretty(&manifest).map_err(|error| {
        NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!("could not encode quarantine manifest: {error}"),
        )
    })?;
    fs::write(&manifest_path, encoded).map_err(|error| {
        NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!("{}: {error}", manifest_path.display()),
        )
    })?;
    mark_read_only(&manifest_path);
    Ok(MovedStore {
        source: orphan.store_dir.clone(),
        quarantined: destination,
        manifest: manifest_path,
    })
}

/// Rebuild `works.json` from the durable event logs of the canonical store and
/// every quarantined store. The quarantined files are read, never written.
fn rebuild_canonical_projection(
    plan: &StoreConsolidationPlan,
    canonical_works: &Path,
    moved: &[MovedStore],
) -> ConsolidationResult<usize> {
    let mut shared = Vec::new();
    if let Some(content) =
        read_optional_string(&gwt_project_state_work_events_path(&plan.canonical_hash))?
    {
        shared.push(content);
    }
    let mut close_parts = Vec::new();
    if let Some(content) =
        read_optional_string(&gwt_workspace_work_events_closed_path(&plan.canonical_hash))?
    {
        close_parts.push(content);
    }
    for entry in moved {
        let store_state = entry.quarantined.join("project-state");
        if let Some(content) = read_optional_string(&store_state.join("work-events.jsonl"))? {
            shared.push(content);
        }
        if let Some(content) = read_optional_string(&store_state.join("work-events-closed.jsonl"))?
        {
            close_parts.push(content);
        }
    }
    let close_content = (!close_parts.is_empty()).then(|| close_parts.join("\n"));

    rebuild_work_events_contents(
        canonical_works,
        shared.iter().map(String::as_str),
        close_content.as_deref(),
    )
    .map_err(|error| {
        NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!("could not rebuild the canonical projection: {error}"),
        )
    })?;

    let readback = load_workspace_work_items_from_path(canonical_works)
        .map_err(|error| {
            NeedsHuman::new(
                StoreConsolidationRefusal::ReadbackFailed,
                format!("{}: {error}", canonical_works.display()),
            )
        })?
        .ok_or_else(|| {
            NeedsHuman::new(
                StoreConsolidationRefusal::ReadbackFailed,
                format!("{} is missing after the rebuild", canonical_works.display()),
            )
        })?;
    Ok(readback.work_items.len())
}

fn rollback(canonical_works: &Path, snapshot: Option<&[u8]>, moved: &[MovedStore]) {
    for entry in moved {
        clear_read_only(&entry.manifest);
        let _ = fs::remove_file(&entry.manifest);
        if entry.quarantined.exists() && !entry.source.exists() {
            let _ = fs::rename(&entry.quarantined, &entry.source);
        }
    }
    match snapshot {
        Some(bytes) => {
            let _ = fs::write(canonical_works, bytes);
        }
        None => {
            let _ = fs::remove_file(canonical_works);
        }
    }
}

/// Exclusive lease over one canonical store, released on drop.
struct WriterLease {
    file: fs::File,
}

impl WriterLease {
    fn acquire(canonical_store: &Path) -> ConsolidationResult<Self> {
        fs::create_dir_all(canonical_store).map_err(|error| {
            NeedsHuman::new(
                StoreConsolidationRefusal::CorruptInput,
                format!("{}: {error}", canonical_store.display()),
            )
        })?;
        let path = canonical_store.join(LEASE_FILE);
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| {
                NeedsHuman::new(
                    StoreConsolidationRefusal::CorruptInput,
                    format!("{}: {error}", path.display()),
                )
            })?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|error| {
            NeedsHuman::new(
                StoreConsolidationRefusal::WriterBusy,
                format!("another consolidation holds {}: {error}", path.display()),
            )
        })?;
        Ok(Self { file })
    }
}

impl Drop for WriterLease {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

fn quarantine_manifest_path(canonical_store: &Path, source_hash: &str) -> PathBuf {
    canonical_store
        .join(QUARANTINE_DIR)
        .join(format!("{source_hash}.json"))
}

fn read_optional_bytes(path: &Path) -> ConsolidationResult<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!("{}: {error}", path.display()),
        )),
    }
}

fn read_optional_string(path: &Path) -> ConsolidationResult<Option<String>> {
    match read_optional_bytes(path)? {
        Some(bytes) => String::from_utf8(bytes).map(Some).map_err(|error| {
            NeedsHuman::new(
                StoreConsolidationRefusal::CorruptInput,
                format!("{} is not UTF-8: {error}", path.display()),
            )
        }),
        None => Ok(None),
    }
}

fn content_revision(bytes: Option<&[u8]>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes.unwrap_or_default());
    hex::encode(hasher.finalize())[..16].to_string()
}

fn compute_manifest_hash(canonical_hash: &str, orphans: &[OrphanedStore]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_hash.as_bytes());
    for orphan in orphans {
        hasher.update(b"\0");
        hasher.update(orphan.source_hash.as_bytes());
        hasher.update(b"\0");
        hasher.update(orphan.revision.as_bytes());
        hasher.update(b"\0");
        hasher.update(orphan.work_item_count.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(orphan.work_event_count.to_string().as_bytes());
    }
    hex::encode(hasher.finalize())[..32].to_string()
}

fn mark_read_only(path: &Path) {
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        let _ = fs::set_permissions(path, permissions);
    }
}

/// Restore owner-writable permissions so a rollback can remove the manifest.
///
/// `set_readonly(false)` would make the file world-writable on Unix, so the
/// mode is set explicitly there and the attribute is cleared on Windows.
fn clear_read_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o644));
    }
    #[cfg(not(unix))]
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
}
