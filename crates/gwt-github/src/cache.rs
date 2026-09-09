//! Local cache for Issue snapshots (SPEC-12 FR-020〜FR-023).
//!
//! The cache is the source of truth for every UI consumer. All reads pass
//! through this layer without touching the network; writes happen only from
//! [`crate::client::IssueClient`] operations that explicitly flow through
//! `pull`-like commands.
//!
//! Filesystem layout (rooted at a configurable directory, typically
//! `~/.gwt/cache/issues/<repo-hash>/`):
//!
//! ```text
//! <root>/
//! └── <issue_number>/
//!     ├── body.md                  # verbatim Issue body
//!     ├── meta.json                # serialized CacheMeta
//!     ├── issue-validation.json    # full-snapshot validation receipt
//!     ├── sections/
//!     │   ├── spec.md              # parsed section content (no markers)
//!     │   ├── tasks.md
//!     │   └── plan.md              # body-inline or assembled from comments
//!     └── comments/
//!         └── <comment_id>.md      # verbatim comment body
//! ```
//!
//! All writes use a tmp-then-rename pattern so concurrent readers never see a
//! half-written file. Directories are created on demand.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, SecondsFormat, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    body::{ParseError, SpecBody, SpecMeta},
    client::{CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt},
    sections::SectionName,
};

pub const ISSUE_VALIDATION_RECEIPT_FILE: &str = "issue-validation.json";
const ISSUE_VALIDATION_RECEIPT_VERSION: u32 = 1;

/// Errors reported by cache operations.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("body parse error: {0}")]
    Parse(#[from] ParseError),
}

/// Serialized metadata stored alongside an Issue body in the cache.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheMeta {
    pub number: u64,
    pub title: String,
    pub labels: Vec<String>,
    pub state: String,
    pub updated_at: String,
    pub comment_ids: Vec<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<String>,
}

impl CacheMeta {
    fn from_snapshot(snapshot: &IssueSnapshot, generation: String) -> Self {
        CacheMeta {
            number: snapshot.number.0,
            title: snapshot.title.clone(),
            labels: snapshot.labels.clone(),
            state: match snapshot.state {
                IssueState::Open => "open".to_string(),
                IssueState::Closed => "closed".to_string(),
            },
            updated_at: snapshot.updated_at.0.clone(),
            comment_ids: snapshot.comments.iter().map(|c| c.id.0).collect(),
            generation: Some(generation),
        }
    }
}

/// A loaded cache entry: the server snapshot plus a parsed [`SpecBody`] view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    pub snapshot: IssueSnapshot,
    pub spec_body: SpecBody,
}

/// Proof that a complete Issue snapshot was validated against GitHub.
///
/// `generation` is opaque to callers. Internally it binds the receipt to the
/// exact persisted snapshot and adds a unique suffix so a reader can detect
/// both mixed-generation reads and ABA-style replacement races.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueValidationReceipt {
    pub version: u32,
    pub generation: String,
    pub validated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheGeneration(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedCacheEntry {
    pub entry: CacheEntry,
    pub generation: Option<CacheGeneration>,
}

/// Result of loading an Issue cache entry together with a stable validation
/// receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatedCacheEntry {
    Missing { generation: Option<CacheGeneration> },
    Unvalidated(VersionedCacheEntry),
    Stale(VersionedCacheEntry),
    Fresh(VersionedCacheEntry),
}

/// Root of the on-disk cache.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// Create a [`Cache`] rooted at the given directory. The directory is
    /// created lazily when the first write occurs.
    pub fn new(root: PathBuf) -> Self {
        Cache { root }
    }

    fn issue_dir(&self, number: IssueNumber) -> PathBuf {
        self.root.join(number.0.to_string())
    }

    pub fn validation_receipt_path(&self, number: IssueNumber) -> PathBuf {
        self.issue_dir(number).join(ISSUE_VALIDATION_RECEIPT_FILE)
    }

    pub fn invalidate_validation_receipt(&self, number: IssueNumber) -> Result<(), CacheError> {
        self.with_issue_lock(number, || {
            self.invalidate_validation_receipt_unlocked(number)
        })
    }

    fn invalidate_validation_receipt_unlocked(
        &self,
        number: IssueNumber,
    ) -> Result<(), CacheError> {
        match fs::remove_file(self.validation_receipt_path(number)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CacheError::Io(error)),
        }
    }

    fn with_issue_lock<T>(
        &self,
        number: IssueNumber,
        action: impl FnOnce() -> Result<T, CacheError>,
    ) -> Result<T, CacheError> {
        let lock_dir = self.root.join(".locks");
        fs::create_dir_all(&lock_dir)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_dir.join(format!("{}.lock", number.0)))?;
        lock.lock_exclusive()?;
        let result = action();
        let unlock_result = FileExt::unlock(&lock).map_err(CacheError::Io);
        match (result, unlock_result) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(value), Ok(())) => Ok(value),
        }
    }

    fn mutate_without_validation<T>(
        &self,
        number: IssueNumber,
        mutation: impl FnOnce() -> Result<T, CacheError>,
    ) -> Result<T, CacheError> {
        self.with_issue_lock(number, || {
            self.mutate_without_validation_unlocked(number, mutation)
        })
    }

    fn mutate_without_validation_unlocked<T>(
        &self,
        number: IssueNumber,
        mutation: impl FnOnce() -> Result<T, CacheError>,
    ) -> Result<T, CacheError> {
        self.invalidate_validation_receipt_unlocked(number)?;
        let result = mutation();
        let final_invalidation = self.invalidate_validation_receipt_unlocked(number);
        match (result, final_invalidation) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(value), Ok(())) => Ok(value),
        }
    }

    /// Load a cache entry only when one stable, content-bound validation
    /// receipt surrounds the read. Missing, malformed, future-dated, or
    /// snapshot-mismatched receipts fail closed.
    pub fn load_validated_entry(
        &self,
        number: IssueNumber,
        ttl: std::time::Duration,
    ) -> Result<ValidatedCacheEntry, CacheError> {
        self.with_issue_lock(number, || {
            let generation = self.current_generation_unlocked(number)?;
            let Some(entry) = self.load_entry(number) else {
                return Ok(ValidatedCacheEntry::Missing { generation });
            };
            let versioned = VersionedCacheEntry { entry, generation };
            let Ok(receipt_bytes) = fs::read(self.validation_receipt_path(number)) else {
                return Ok(ValidatedCacheEntry::Unvalidated(versioned));
            };
            let Ok(receipt) = serde_json::from_slice::<IssueValidationReceipt>(&receipt_bytes)
            else {
                return Ok(ValidatedCacheEntry::Unvalidated(versioned));
            };
            if receipt.version != ISSUE_VALIDATION_RECEIPT_VERSION
                || receipt
                    .generation
                    .split_once(':')
                    .filter(|(_, validation_generation)| !validation_generation.is_empty())
                    .map(|(cache_generation, _)| cache_generation)
                    != versioned.generation.as_ref().map(|value| value.0.as_str())
            {
                return Ok(ValidatedCacheEntry::Unvalidated(versioned));
            }
            let Ok(validated_at) = DateTime::parse_from_rfc3339(&receipt.validated_at) else {
                return Ok(ValidatedCacheEntry::Unvalidated(versioned));
            };
            let age = Utc::now().signed_duration_since(validated_at.with_timezone(&Utc));
            let Ok(ttl) = chrono::Duration::from_std(ttl) else {
                return Ok(ValidatedCacheEntry::Stale(versioned));
            };
            if age < chrono::Duration::zero() || age >= ttl {
                Ok(ValidatedCacheEntry::Stale(versioned))
            } else {
                Ok(ValidatedCacheEntry::Fresh(versioned))
            }
        })
    }

    /// Renew the full-snapshot validation receipt only if the persisted cache
    /// still matches the snapshot that was just validated.
    pub fn renew_validation_receipt_if_current(
        &self,
        expected: &IssueSnapshot,
    ) -> Result<bool, CacheError> {
        self.with_issue_lock(expected.number, || {
            let generation = self.current_generation_unlocked(expected.number)?;
            self.renew_validation_receipt_unlocked(expected, generation.as_ref())
        })
    }

    pub fn renew_validation_receipt_if_generation(
        &self,
        expected: &IssueSnapshot,
        generation: Option<&CacheGeneration>,
    ) -> Result<bool, CacheError> {
        self.with_issue_lock(expected.number, || {
            self.renew_validation_receipt_unlocked(expected, generation)
        })
    }

    fn renew_validation_receipt_unlocked(
        &self,
        expected: &IssueSnapshot,
        generation: Option<&CacheGeneration>,
    ) -> Result<bool, CacheError> {
        if self.current_generation_unlocked(expected.number)?.as_ref() != generation {
            return Ok(false);
        }
        let Some(current) = self.load_entry(expected.number) else {
            return Ok(false);
        };
        if !persisted_snapshots_match(&current.snapshot, expected) {
            return Ok(false);
        }
        let Some(generation) = generation else {
            return Ok(false);
        };
        let receipt = IssueValidationReceipt {
            version: ISSUE_VALIDATION_RECEIPT_VERSION,
            generation: format!("{}:{}", generation.0, Uuid::new_v4()),
            validated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true),
        };
        self.invalidate_validation_receipt_unlocked(expected.number)?;
        write_atomic(
            &self.validation_receipt_path(expected.number),
            &serde_json::to_vec_pretty(&receipt)?,
        )?;
        Ok(true)
    }

    pub fn current_generation(
        &self,
        number: IssueNumber,
    ) -> Result<Option<CacheGeneration>, CacheError> {
        self.with_issue_lock(number, || self.current_generation_unlocked(number))
    }

    fn current_generation_unlocked(
        &self,
        number: IssueNumber,
    ) -> Result<Option<CacheGeneration>, CacheError> {
        let meta_path = self.issue_dir(number).join("meta.json");
        let bytes = match fs::read(meta_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(CacheError::Io(error)),
        };
        let meta: CacheMeta = match serde_json::from_slice(&bytes) {
            Ok(meta) => meta,
            // A malformed metadata file cannot supply a trustworthy
            // generation. Treat it like a legacy/unvalidated entry so an
            // unconditional remote fetch can atomically self-repair it.
            Err(_) => return Ok(None),
        };
        Ok(meta.generation.map(CacheGeneration))
    }

    /// Write a full Issue snapshot to the cache atomically.
    ///
    /// After writing, the `sections/` and `comments/` directories are
    /// swept so that files belonging to sections or comments that no
    /// longer exist in `snapshot` are deleted. Without this sweep a
    /// SPEC would grow monotonically across edits even after sections
    /// were removed or re-routed from the body into comments (or vice
    /// versa), and stale reads from `read_section` would return
    /// content the Issue has already deleted.
    pub fn write_snapshot(&self, snapshot: &IssueSnapshot) -> Result<(), CacheError> {
        self.mutate_without_validation(snapshot.number, || {
            self.write_snapshot_files_unlocked(
                snapshot,
                CacheGeneration(Uuid::new_v4().to_string()),
            )
        })
    }

    /// Commit a remotely fetched snapshot only if no cache writer has changed
    /// the Issue since the caller inspected it.
    pub fn write_snapshot_if_generation(
        &self,
        snapshot: &IssueSnapshot,
        expected: Option<&CacheGeneration>,
    ) -> Result<Option<CacheGeneration>, CacheError> {
        self.with_issue_lock(snapshot.number, || {
            if self.current_generation_unlocked(snapshot.number)?.as_ref() != expected {
                return Ok(None);
            }
            let committed_generation = CacheGeneration(Uuid::new_v4().to_string());
            self.mutate_without_validation_unlocked(snapshot.number, || {
                self.write_snapshot_files_unlocked(snapshot, committed_generation.clone())
            })?;
            Ok(Some(committed_generation))
        })
    }

    fn write_snapshot_files_unlocked(
        &self,
        snapshot: &IssueSnapshot,
        generation: CacheGeneration,
    ) -> Result<(), CacheError> {
        use std::collections::HashSet;

        let dir = self.issue_dir(snapshot.number);
        let sections_dir = dir.join("sections");
        let comments_dir = dir.join("comments");
        fs::create_dir_all(&sections_dir)?;
        fs::create_dir_all(&comments_dir)?;

        // Write body.md (tmp -> rename).
        write_atomic(&dir.join("body.md"), snapshot.body.as_bytes())?;

        // Collect the comment filenames that this snapshot asserts
        // should exist, write them, and then sweep any leftover
        // `comments/*.md` that isn't in the set.
        let mut desired_comments: HashSet<String> = HashSet::new();
        for comment in &snapshot.comments {
            let filename = format!("{}.md", comment.id.0);
            let path = comments_dir.join(&filename);
            write_atomic(&path, comment.body.as_bytes())?;
            desired_comments.insert(filename);
        }
        prune_unlisted_files(&comments_dir, &desired_comments)?;

        // Parse the body + comments into a SpecBody and write per-section files.
        let parsed_comments: Vec<crate::body::Comment> = snapshot
            .comments
            .iter()
            .map(|c| crate::body::Comment {
                id: c.id.0,
                body: c.body.clone(),
            })
            .collect();
        match SpecBody::parse(&snapshot.body, &parsed_comments) {
            Ok(spec_body) => {
                let mut desired_sections: HashSet<String> = HashSet::new();
                for (name, content) in &spec_body.sections {
                    let filename = section_filename(name);
                    let path = sections_dir.join(&filename);
                    write_atomic(&path, content.as_bytes())?;
                    desired_sections.insert(filename);
                }
                prune_unlisted_files(&sections_dir, &desired_sections)?;
            }
            Err(_) => {
                // Either the body has no gwt-spec header at all (plain Issue),
                // or it looks like a SPEC but the structural parse failed
                // (e.g., the body contains prose / code that happens to match
                // the gwt-spec header pattern, the sections index is malformed,
                // or a referenced comment is missing). In all such cases the
                // Issue cannot be safely exposed as a structured SPEC, so we
                // persist it as a plain Issue: body and meta are cached, but
                // no per-section files are written. A subsequent refresh that
                // arrives with a well-formed SPEC body will rewrite the
                // sections/ tree on the next call.
                let desired_sections: HashSet<String> = HashSet::new();
                prune_unlisted_files(&sections_dir, &desired_sections)?;
            }
        }

        // Finally publish the new generation in meta.json.
        let meta = CacheMeta::from_snapshot(snapshot, generation.0);
        let meta_bytes = serde_json::to_vec_pretty(&meta)?;
        write_atomic(&dir.join("meta.json"), &meta_bytes)?;
        Ok(())
    }

    /// Load a full cache entry by issue number. Returns `None` if the issue
    /// is not present in the cache.
    pub fn load_entry(&self, number: IssueNumber) -> Option<CacheEntry> {
        let dir = self.issue_dir(number);
        if !dir.is_dir() {
            return None;
        }
        let body = fs::read_to_string(dir.join("body.md")).ok()?;
        let meta_bytes = fs::read(dir.join("meta.json")).ok()?;
        let meta: CacheMeta = serde_json::from_slice(&meta_bytes).ok()?;

        // Re-hydrate comment snapshots from the comments/ directory.
        let mut comments: Vec<CommentSnapshot> = Vec::new();
        let comments_dir = dir.join("comments");
        if comments_dir.is_dir() {
            for cid in &meta.comment_ids {
                let path = comments_dir.join(format!("{cid}.md"));
                if let Ok(body) = fs::read_to_string(&path) {
                    comments.push(CommentSnapshot {
                        id: CommentId(*cid),
                        body,
                        // We do not persist per-comment updated_at in meta; the
                        // issue-level updated_at is the authoritative cache key
                        // for conditional fetches.
                        updated_at: UpdatedAt::new(meta.updated_at.clone()),
                    });
                }
            }
        }

        let snapshot = IssueSnapshot {
            number: IssueNumber(meta.number),
            title: meta.title.clone(),
            body,
            labels: meta.labels.clone(),
            state: match meta.state.as_str() {
                "closed" => IssueState::Closed,
                _ => IssueState::Open,
            },
            updated_at: UpdatedAt::new(meta.updated_at.clone()),
            comments,
        };

        let parsed_comments: Vec<crate::body::Comment> = snapshot
            .comments
            .iter()
            .map(|c| crate::body::Comment {
                id: c.id.0,
                body: c.body.clone(),
            })
            .collect();
        let spec_body = match SpecBody::parse(&snapshot.body, &parsed_comments) {
            Ok(spec_body) => spec_body,
            Err(ParseError::MissingHeader) => SpecBody {
                // Plain Issue (no `<!-- gwt-spec id=... -->` header at all):
                // synthesize an empty SpecBody so the entry still surfaces
                // to UI consumers as a regular Issue. This mirrors the
                // existing `write_snapshot` path for plain Issues.
                meta: SpecMeta {
                    id: meta.number.to_string(),
                    version: 1,
                },
                sections_index: crate::body::SectionsIndex::default(),
                sections: std::collections::BTreeMap::new(),
            },
            Err(_) => {
                // Body carries a SPEC header but the structural parse fails
                // (malformed sections index, missing referenced comment,
                // etc.). We intentionally do NOT downgrade these to an
                // empty SpecBody: a subsequent `SpecOps::write_section`
                // would recompute the routing from the empty section map
                // and rewrite the body's index, orphaning content stored in
                // comments referenced only by the original (malformed)
                // index. Returning `None` keeps such entries out of UI
                // lists until the next refresh either repairs the body or
                // proves it is truly a plain Issue. The on-disk cache is
                // still populated (write_snapshot is lenient), so the
                // body / meta survive in `~/.gwt/cache/issues/<n>/` for
                // diagnostics.
                return None;
            }
        };
        Some(CacheEntry {
            snapshot,
            spec_body,
        })
    }

    /// List every readable cache entry stored under the cache root.
    ///
    /// Non-directory entries, non-numeric directory names, and partially
    /// unreadable cache entries are skipped so UI consumers can stay on the
    /// typed cache surface rather than reimplementing the on-disk layout.
    pub fn list_entries(&self) -> Result<Vec<CacheEntry>, CacheError> {
        let entries = fs::read_dir(&self.root)?;
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Ok(number) = name.parse::<u64>() else {
                continue;
            };
            if let Some(cache_entry) = self.load_entry(IssueNumber(number)) {
                out.push(cache_entry);
            }
        }
        Ok(out)
    }

    /// Read a single section by name. Returns `Ok(None)` if the section is
    /// absent from the cache.
    pub fn read_section(
        &self,
        number: IssueNumber,
        name: &SectionName,
    ) -> Result<Option<String>, CacheError> {
        let path = self
            .issue_dir(number)
            .join("sections")
            .join(section_filename(name));
        match fs::read_to_string(&path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(CacheError::Io(e)),
        }
    }

    /// SPEC-2017 T-008 — Atomically rewrite the `labels` array on the
    /// cached `meta.json` for `number`.
    ///
    /// This is the local mirror of a Kanban phase change: after the
    /// frontend D&D pushes the new labels through the GitHub API, the
    /// cache needs to reflect the same labels so the next render shows
    /// the card in the right column without waiting for a full refresh.
    ///
    /// `body.md`, `sections/*`, and `comments/*` are intentionally left
    /// alone — only the labels array is rewritten. Returns
    /// [`CacheError::Io`] when the entry is missing on disk so the
    /// caller can surface a typed error instead of silently succeeding.
    pub fn apply_phase_change(
        &self,
        number: IssueNumber,
        new_labels: Vec<String>,
    ) -> Result<(), CacheError> {
        self.mutate_without_validation(number, || {
            let meta_path = self.issue_dir(number).join("meta.json");
            let meta_bytes = match fs::read(&meta_path) {
                Ok(bytes) => bytes,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    return Err(CacheError::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("issue #{} not in cache", number.0),
                    )));
                }
                Err(err) => return Err(CacheError::Io(err)),
            };
            let mut meta: CacheMeta = serde_json::from_slice(&meta_bytes)?;
            meta.labels = new_labels;
            meta.generation = Some(Uuid::new_v4().to_string());
            let updated_bytes = serde_json::to_vec_pretty(&meta)?;
            write_atomic(&meta_path, &updated_bytes)?;
            Ok(())
        })
    }
}

fn persisted_snapshots_match(left: &IssueSnapshot, right: &IssueSnapshot) -> bool {
    left.number == right.number
        && left.title == right.title
        && left.body == right.body
        && left.labels == right.labels
        && left.state == right.state
        && left.updated_at == right.updated_at
        && left.comments.len() == right.comments.len()
        && left
            .comments
            .iter()
            .zip(&right.comments)
            .all(|(left, right)| left.id == right.id && left.body == right.body)
}

/// Write bytes to `path` atomically via a `.tmp-<pid>-<nanos>` sibling file
/// followed by `rename`.
///
/// Exposed (via `cache::write_atomic`) so that other crates in the workspace
/// — notably gwt's hook handlers (SPEC #1942) — can reuse the exact
/// same crash-safe write path for state files like `runtime-state.json`.
/// Not part of the semver-stable surface; `#[doc(hidden)]` keeps it out of
/// generated docs but `pub` is required so the hook code can link against it.
#[doc(hidden)]
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().expect("path must have a parent");
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name().unwrap().to_string_lossy(),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Best-effort cleanup of the tmp file on failure.
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Remove every regular file under `dir` whose filename is not in
/// `keep`. Subdirectories and tmp files (`.*.tmp-*`) are left alone.
/// Errors encountered while reading the directory are propagated;
/// errors from individual `remove_file` calls are also propagated so
/// that a broken cache surfaces loudly instead of silently drifting.
fn prune_unlisted_files(
    dir: &Path,
    keep: &std::collections::HashSet<String>,
) -> std::io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let entry = entry?;
        let name_os = entry.file_name();
        let Some(name) = name_os.to_str() else {
            continue;
        };
        // Skip the write-atomic tmp staging files — they are
        // transient and deleting them here would race the in-progress
        // writers.
        if name.starts_with('.') {
            continue;
        }
        if !entry.file_type()?.is_file() {
            continue;
        }
        if !keep.contains(name) {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Map a [`SectionName`] to a safe-ish filename under `sections/`. We keep
/// slashes by replacing them with `__` so that `contract/api.yaml` lands at
/// `sections/contract__api.yaml.md`.
fn section_filename(name: &SectionName) -> String {
    let sanitized = name.0.replace('/', "__");
    if sanitized.ends_with(".md") {
        sanitized
    } else {
        format!("{sanitized}.md")
    }
}
