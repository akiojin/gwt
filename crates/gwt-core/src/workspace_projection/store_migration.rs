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
//! 2. [`apply_store_consolidation`] takes the *real* Work projection lock of
//!    the canonical store and of every store it is about to absorb, refuses
//!    unless the manifest hash still matches, *moves* each orphan under
//!    `quarantine/` beside a read-only manifest, and rebuilds the canonical
//!    projection from the durable event logs plus every eventless legacy row.
//!    It reads the result back and rolls everything back if that fails.
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
    work_events_intake::rebuild_work_events_contents_locked,
    workspace_projection::load_workspace_work_items_from_path,
};

/// Schema version of [`QuarantineManifest`].
pub const QUARANTINE_MANIFEST_VERSION: u32 = 1;

/// Schema version of [`IssuedConsolidationPlan`].
pub const ISSUED_PLAN_VERSION: u32 = 1;

const QUARANTINE_DIR: &str = "quarantine";
const ISSUED_PLAN_FILE: &str = "store-consolidation-plan.json";

/// Why a consolidation stopped. Every variant needs a human decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreConsolidationRefusal {
    /// No repository identity could be resolved, so there is no canonical
    /// store to consolidate into.
    UnknownRemote,
    /// A live Work writer holds the projection lock of a store this
    /// consolidation needs.
    WriterBusy,
    /// The disk no longer matches the plan the caller approved.
    ManifestChanged,
    /// No reviewed dry run issued the plan this apply claims to have approved.
    PlanNotIssued,
    /// A store could not be read or parsed.
    CorruptInput,
    /// The rebuilt projection did not read back intact; changes were rolled
    /// back.
    ReadbackFailed,
}

impl StoreConsolidationRefusal {
    /// Stable machine-readable reason code. Callers key on this, never on the
    /// human-readable detail.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownRemote => "unknown_remote",
            Self::WriterBusy => "writer_busy",
            Self::ManifestChanged => "manifest_changed",
            Self::PlanNotIssued => "plan_not_issued",
            Self::CorruptInput => "corrupt_input",
            Self::ReadbackFailed => "readback_failed",
        }
    }

    /// Whether repeating the same request could succeed without a human first
    /// changing something. A busy writer clears on its own; a plan the disk no
    /// longer matches needs a fresh reviewed dry run, which the caller can do.
    /// Corrupt input and a failed readback need a person to look.
    pub fn retryable(self) -> bool {
        match self {
            Self::WriterBusy | Self::ManifestChanged | Self::PlanNotIssued => true,
            Self::UnknownRemote | Self::CorruptInput | Self::ReadbackFailed => false,
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

/// The record a reviewed dry run leaves behind so an apply can prove it was
/// preceded by one.
///
/// Issue #3524 (folded into #3606): the review gate used to be nothing but a
/// hash comparison, and the refusal text handed the caller the current hash —
/// so "apply without reading the plan" was a two-call loop. Apply now requires
/// a hash that a dry run actually issued, and no refusal ever quotes a valid
/// one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedConsolidationPlan {
    pub version: u32,
    pub manifest_hash: String,
    pub canonical_hash: String,
    pub project_root: PathBuf,
    pub issued_at: DateTime<Utc>,
    /// The Session that reviewed the plan, when the issuer had one. An apply
    /// from a different Session is refused.
    pub issued_by_session: Option<String>,
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

/// Record that `plan` was reviewed, so [`apply_store_consolidation`] can prove
/// the apply was preceded by a dry run.
///
/// Writing this is the reviewing caller's deliberate act; planning itself stays
/// side-effect free so a dry run never materializes the canonical store.
pub fn issue_store_consolidation_plan(
    plan: &StoreConsolidationPlan,
    issued_by_session: Option<&str>,
) -> ConsolidationResult<IssuedConsolidationPlan> {
    let issued = IssuedConsolidationPlan {
        version: ISSUED_PLAN_VERSION,
        manifest_hash: plan.manifest_hash.clone(),
        canonical_hash: plan.canonical_hash.as_str().to_string(),
        project_root: plan.project_root.clone(),
        issued_at: Utc::now(),
        issued_by_session: issued_by_session
            .map(str::trim)
            .filter(|session| !session.is_empty())
            .map(str::to_string),
    };
    let path = issued_plan_path(&plan.canonical_store);
    let parent = path.parent().expect("issued plan parent");
    fs::create_dir_all(parent).map_err(|error| {
        NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!("{}: {error}", parent.display()),
        )
    })?;
    let encoded = serde_json::to_vec_pretty(&issued).map_err(|error| {
        NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!("could not encode the issued plan: {error}"),
        )
    })?;
    fs::write(&path, encoded).map_err(|error| {
        NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!("{}: {error}", path.display()),
        )
    })?;
    Ok(issued)
}

/// The plan a dry run last issued for `canonical_store`, if any.
pub fn issued_store_consolidation_plan(
    canonical_store: &Path,
) -> ConsolidationResult<Option<IssuedConsolidationPlan>> {
    let path = issued_plan_path(canonical_store);
    let Some(bytes) = read_optional_bytes(&path)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!("{} is not a readable issued plan: {error}", path.display()),
        )
    })
}

fn issued_plan_path(canonical_store: &Path) -> PathBuf {
    canonical_store.join("project-state").join(ISSUED_PLAN_FILE)
}

/// Quarantine every orphaned store and rebuild the canonical projection.
///
/// `expected_manifest_hash` is the [`StoreConsolidationPlan::manifest_hash`]
/// the caller reviewed, and a dry run must have issued it through
/// [`issue_store_consolidation_plan`]. `session_id` is the Session applying the
/// plan; it must match the one that reviewed it. Applying the same plan twice
/// is a no-op.
pub fn apply_store_consolidation(
    project_root: &Path,
    expected_manifest_hash: &str,
    session_id: Option<&str>,
) -> ConsolidationResult<StoreConsolidationOutcome> {
    let plan = plan_store_consolidation(project_root)?;
    if plan.manifest_hash != expected_manifest_hash {
        // Deliberately silent about the current hash: quoting it here is what
        // let a caller apply a plan it never read (#3524).
        return Err(NeedsHuman::new(
            StoreConsolidationRefusal::ManifestChanged,
            "the store changed since the approved plan; re-run the dry run and \
             review the new plan before applying"
                .to_string(),
        ));
    }
    if plan.is_empty() {
        return Ok(StoreConsolidationOutcome::NothingToDo);
    }
    verify_plan_was_issued(&plan, expected_manifest_hash, session_id)?;

    // Issue #3524 (folded into #3606): contend on the lock the Work
    // persistence itself takes, for the canonical store *and* every store about
    // to be absorbed. A private consolidation lease left a live writer free to
    // be mid-write in a store that this function then renamed away. The locks
    // are tried, never waited on, so a busy project is a bounded refusal rather
    // than a migration parked behind whatever writer happens to be running.
    let mut lock_targets = vec![work_items_lock_path(&plan.canonical_store)];
    lock_targets.extend(
        plan.orphans
            .iter()
            .map(|orphan| work_items_lock_path(&orphan.store_dir)),
    );
    let _leases = WorkStoreLeases::acquire(lock_targets)?;

    // Re-plan under the locks: a writer may have moved between the dry run and
    // now, and the locks are what make this observation stable.
    let confirmed = plan_store_consolidation(project_root)?;
    if confirmed.manifest_hash != expected_manifest_hash {
        return Err(NeedsHuman::new(
            StoreConsolidationRefusal::ManifestChanged,
            "the store changed while the projection locks were being acquired; \
             re-run the dry run and review the new plan before applying"
                .to_string(),
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

/// Refuse an apply that no reviewed dry run stands behind.
///
/// The hash alone is not enough evidence: a caller can obtain a matching hash
/// without ever reading what it authorizes. The issued record is what proves a
/// human-visible dry run produced this exact plan, for this project, in this
/// Session.
fn verify_plan_was_issued(
    plan: &StoreConsolidationPlan,
    expected_manifest_hash: &str,
    session_id: Option<&str>,
) -> ConsolidationResult<()> {
    let Some(issued) = issued_store_consolidation_plan(&plan.canonical_store)? else {
        return Err(NeedsHuman::new(
            StoreConsolidationRefusal::PlanNotIssued,
            "no reviewed dry run has been recorded for this project; run \
             workspace.store_consolidate with dry_run true and review the plan first"
                .to_string(),
        ));
    };
    if issued.version != ISSUED_PLAN_VERSION {
        return Err(NeedsHuman::new(
            StoreConsolidationRefusal::PlanNotIssued,
            format!(
                "the recorded dry run uses schema version {} instead of {ISSUED_PLAN_VERSION}; \
                 re-run the dry run",
                issued.version
            ),
        ));
    }
    if issued.manifest_hash != expected_manifest_hash
        || issued.canonical_hash != plan.canonical_hash.as_str()
    {
        return Err(NeedsHuman::new(
            StoreConsolidationRefusal::PlanNotIssued,
            "the approved manifest does not match the plan the last dry run issued; \
             re-run the dry run and review the new plan"
                .to_string(),
        ));
    }
    let session_id = session_id
        .map(str::trim)
        .filter(|session| !session.is_empty());
    if issued.issued_by_session.as_deref() != session_id {
        return Err(NeedsHuman::new(
            StoreConsolidationRefusal::PlanNotIssued,
            "the plan was reviewed by a different Session; re-run the dry run in this Session"
                .to_string(),
        ));
    }
    Ok(())
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
///
/// Issue #3524 (folded into #3606): the event logs are not the whole story. The
/// real split store held 796 Work rows against 240 events, because rows
/// predating event recording live only in `works.json`. Those eventless rows
/// are carried across as legacy metadata, and the readback then proves that
/// every Work id present in any input still exists — losing one silently would
/// be indistinguishable from a successful migration.
fn rebuild_canonical_projection(
    plan: &StoreConsolidationPlan,
    canonical_works: &Path,
    moved: &[MovedStore],
) -> ConsolidationResult<usize> {
    let mut expected_ids: HashSet<String> = work_item_ids(canonical_works)?;
    let mut legacy_items = Vec::new();
    for entry in moved {
        let orphan_works = entry.quarantined.join("project-state").join("works.json");
        let Some(projection) = load_store_projection(&orphan_works)? else {
            continue;
        };
        expected_ids.extend(projection.work_items.iter().map(|item| item.id.clone()));
        legacy_items.extend(
            projection
                .work_items
                .into_iter()
                .filter(|item| item.events.is_empty() || item.legacy_metadata_authoritative),
        );
    }

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

    // The projection locks for every store involved are already held by the
    // caller, so this must use the lock-free rebuild: re-entering the lock from
    // inside the transaction would deadlock against the caller's own lock.
    rebuild_work_events_contents_locked(
        canonical_works,
        shared.iter().map(String::as_str),
        close_content.as_deref(),
        legacy_items,
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

    let rebuilt_ids: HashSet<&str> = readback
        .work_items
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    let mut lost: Vec<&str> = expected_ids
        .iter()
        .map(String::as_str)
        .filter(|id| !rebuilt_ids.contains(id))
        .collect();
    if !lost.is_empty() {
        lost.sort_unstable();
        lost.truncate(20);
        return Err(NeedsHuman::new(
            StoreConsolidationRefusal::ReadbackFailed,
            format!(
                "the rebuilt projection lost {} Work id(s) present in the inputs: {}",
                expected_ids.len() - rebuilt_ids.len().min(expected_ids.len()),
                lost.join(", ")
            ),
        ));
    }
    Ok(readback.work_items.len())
}

/// Work ids currently recorded in one store's projection. A missing file is an
/// empty set; an unreadable one is corrupt input.
fn work_item_ids(works_path: &Path) -> ConsolidationResult<HashSet<String>> {
    Ok(load_store_projection(works_path)?
        .map(|projection| {
            projection
                .work_items
                .into_iter()
                .map(|item| item.id)
                .collect()
        })
        .unwrap_or_default())
}

fn load_store_projection(
    works_path: &Path,
) -> ConsolidationResult<Option<crate::workspace_projection::WorkItemsProjection>> {
    if !works_path.exists() {
        return Ok(None);
    }
    load_workspace_work_items_from_path(works_path).map_err(|error| {
        NeedsHuman::new(
            StoreConsolidationRefusal::CorruptInput,
            format!("{}: {error}", works_path.display()),
        )
    })
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

/// The lock a live Work writer takes for one project store's projection.
///
/// `works.lock` beside `works.json` is exactly what
/// `workspace_projection::persistence` locks, so contending here is contending
/// with the real writers rather than with a lease only this module knows about.
fn work_items_lock_path(store_dir: &Path) -> PathBuf {
    store_dir.join("project-state").join("works.lock")
}

/// Exclusive locks over every project store a consolidation touches, released
/// on drop. Acquired in sorted order so two consolidations can never deadlock
/// against each other, and tried rather than waited on so a live writer is a
/// refusal instead of an unbounded stall.
struct WorkStoreLeases {
    files: Vec<fs::File>,
}

impl WorkStoreLeases {
    fn acquire(lock_paths: Vec<PathBuf>) -> ConsolidationResult<Self> {
        let mut lock_paths = lock_paths;
        lock_paths.sort();
        lock_paths.dedup();
        let mut files = Vec::with_capacity(lock_paths.len());
        for path in lock_paths {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    NeedsHuman::new(
                        StoreConsolidationRefusal::CorruptInput,
                        format!("{}: {error}", parent.display()),
                    )
                })?;
            }
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
                    format!("a Work writer holds {}: {error}", path.display()),
                )
            })?;
            files.push(file);
        }
        Ok(Self { files })
    }
}

impl Drop for WorkStoreLeases {
    fn drop(&mut self) {
        for file in &self.files {
            let _ = fs2::FileExt::unlock(file);
        }
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
