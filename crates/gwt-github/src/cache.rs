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
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
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
const SUMMARY_GENERATION_FILE: &str = ".summary-generation";
const SUMMARY_CACHE_ROOT_LIMIT: usize = 8;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_login: Option<String>,
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
            author_login: snapshot.author_login.clone(),
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
    /// Set when the body carries a gwt-spec header but the SPEC structure
    /// cannot be parsed. `spec_body` is then empty and must not drive a
    /// section write (Issue #4392).
    pub spec_parse_error: Option<String>,
}

/// Issue fields needed by lists, without comment hydration or SPEC parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSummary {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub state: IssueState,
    pub updated_at: String,
}

impl From<&IssueSnapshot> for IssueSummary {
    fn from(snapshot: &IssueSnapshot) -> Self {
        Self {
            number: snapshot.number.0,
            title: snapshot.title.clone(),
            body: snapshot.body.clone(),
            labels: snapshot.labels.clone(),
            state: snapshot.state,
            updated_at: snapshot.updated_at.0.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryFileStamp {
    modified: SystemTime,
    created: Option<SystemTime>,
    length: u64,
}

impl SummaryFileStamp {
    fn read(path: &Path) -> std::io::Result<Option<Self>> {
        match fs::metadata(path) {
            Ok(metadata) => Ok(Some(Self {
                modified: metadata.modified()?,
                created: metadata.created().ok(),
                length: metadata.len(),
            })),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryCacheKey {
    root: SummaryFileStamp,
    generation: Option<SummaryFileStamp>,
}

struct SummaryCacheEntry {
    root: PathBuf,
    key: SummaryCacheKey,
    summaries: Arc<Vec<IssueSummary>>,
}

fn summary_cache() -> &'static Mutex<Vec<SummaryCacheEntry>> {
    static CACHE: OnceLock<Mutex<Vec<SummaryCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
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

/// Why a validation-receipt renewal did or did not publish a receipt.
///
/// Issue #4436 AC-4: the renewal used to answer with a bare `bool`, and every
/// caller reported `false` as "the cache changed under this read; retry". Only
/// one of the four ways it can decline is a concurrent cache writer. The other
/// three are properties of the persisted entry itself, so they never clear on a
/// retry — an operator trying to repair a broken Issue got the same "retry"
/// advice five times in a row while `meta.json` never moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationReceiptRenewal {
    /// A receipt now binds the caller's snapshot to the persisted entry.
    Renewed,
    /// Another cache writer replaced the Issue since the caller read it. This
    /// is the only outcome a retry can resolve.
    GenerationChanged,
    /// The caller held no generation, so there is nothing to bind a receipt to.
    GenerationMissing,
    /// The persisted entry could not be read back at all.
    EntryUnreadable,
    /// The persisted entry still carries the caller's generation but does not
    /// round-trip to the snapshot that was validated.
    SnapshotMismatch,
}

impl ValidationReceiptRenewal {
    /// Whether a receipt was published.
    #[must_use]
    pub fn renewed(self) -> bool {
        matches!(self, Self::Renewed)
    }

    /// Whether a concurrent cache writer explains the refusal. False for every
    /// outcome a retry cannot change.
    #[must_use]
    pub fn cache_changed(self) -> bool {
        matches!(self, Self::GenerationChanged)
    }

    /// The operator-facing reason, without any retry advice.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Renewed => "the validation receipt was published",
            Self::GenerationChanged => {
                "another cache writer replaced the snapshot during this read"
            }
            Self::GenerationMissing => "the read held no cache generation to bind a receipt to",
            Self::EntryUnreadable => "the persisted cache entry could not be read back",
            Self::SnapshotMismatch => {
                "the persisted cache entry does not match the validated snapshot"
            }
        }
    }

    /// The operation that recovers this state.
    ///
    /// Issue #4392 AC-4: [`Self::reason`] says what went wrong, which is not
    /// the same as saying what to do. An operator who reached an outcome a
    /// retry cannot clear was told only that a retry cannot clear it, and the
    /// Issue stayed unreadable. Empty for [`Self::Renewed`], which is not a
    /// refusal.
    #[must_use]
    pub fn remedy(self) -> &'static str {
        match self {
            Self::Renewed => "",
            Self::GenerationChanged => "retry the operation",
            Self::GenerationMissing | Self::EntryUnreadable | Self::SnapshotMismatch => {
                "run issue.cache.repair for this Issue to rewrite the cache entry and republish \
                 its receipt"
            }
        }
    }
}

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

    /// Serialize host-local SPEC writers across processes. Kept separate from
    /// the cache lock because a remote transaction refreshes the cache itself.
    pub(crate) fn lock_spec_write(&self, number: IssueNumber) -> Result<fs::File, CacheError> {
        let directory = self.root.join(".locks");
        fs::create_dir_all(&directory)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(directory.join(format!("{}.spec-write.lock", number.0)))?;
        lock.lock_exclusive()?;
        Ok(lock)
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
        self.invalidate_summaries()?;
        self.invalidate_validation_receipt_unlocked(number)?;
        let before = self.load_entry(number);
        let result = mutation();
        let final_invalidation = self.invalidate_validation_receipt_unlocked(number);
        let summary_invalidation = self.invalidate_summaries();
        let after = self.load_entry(number);
        let unchanged = match (&before, &after) {
            (Some(before), Some(after)) => {
                persisted_snapshots_match(&before.snapshot, &after.snapshot)
            }
            (None, None) => true,
            _ => false,
        };
        // Publish after the mutation, including partial failures. Receipt and
        // per-Issue generation renewal alone do not invalidate list readers.
        if !unchanged || result.is_err() {
            write_atomic(
                &self.root.join("source-revision"),
                Uuid::new_v4().to_string().as_bytes(),
            )?;
        }
        match (result, final_invalidation.and(summary_invalidation)) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(value), Ok(())) => Ok(value),
        }
    }

    /// O(1) source revision for list readers, shared across cache writers.
    /// Legacy caches start at the empty revision until their first mutation.
    pub fn revision(&self) -> Result<String, CacheError> {
        match fs::read_to_string(self.root.join("source-revision")) {
            Ok(revision) => Ok(revision),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error.into()),
        }
    }

    fn invalidate_summaries(&self) -> Result<(), CacheError> {
        summary_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|entry| entry.root != self.root);
        // Every canonical writer publishes a new disk stamp, including edits
        // inside an existing issue directory that do not change root mtime.
        write_atomic_with_durability(
            &self.root.join(SUMMARY_GENERATION_FILE),
            Uuid::new_v4().to_string().as_bytes(),
            Durability::RenameOnly,
        )?;
        Ok(())
    }

    fn summary_cache_key(&self) -> Result<Option<SummaryCacheKey>, CacheError> {
        let Some(root) = SummaryFileStamp::read(&self.root)? else {
            return Ok(None);
        };
        Ok(Some(SummaryCacheKey {
            root,
            generation: SummaryFileStamp::read(&self.root.join(SUMMARY_GENERATION_FILE))?,
        }))
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
    ) -> Result<ValidationReceiptRenewal, CacheError> {
        self.with_issue_lock(expected.number, || {
            let generation = self.current_generation_unlocked(expected.number)?;
            self.renew_validation_receipt_unlocked(expected, generation.as_ref())
        })
    }

    pub fn renew_validation_receipt_if_generation(
        &self,
        expected: &IssueSnapshot,
        generation: Option<&CacheGeneration>,
    ) -> Result<ValidationReceiptRenewal, CacheError> {
        self.with_issue_lock(expected.number, || {
            self.renew_validation_receipt_unlocked(expected, generation)
        })
    }

    /// Commit a snapshot and publish its validation receipt under one lock.
    ///
    /// Issue #4392 AC-3: `write_snapshot` routes through the private
    /// `mutate_without_validation` helper, which deletes the receipt, so every
    /// writer that used it left the entry unproven. Without a primitive that
    /// does both, the only way to obtain a receipt was a full read path, and an
    /// entry that path refused could not be repaired at all.
    pub fn write_snapshot_with_receipt(
        &self,
        snapshot: &IssueSnapshot,
    ) -> Result<ValidationReceiptRenewal, CacheError> {
        self.with_issue_lock(snapshot.number, || {
            let generation = CacheGeneration(Uuid::new_v4().to_string());
            self.mutate_without_validation_unlocked(snapshot.number, || {
                self.write_snapshot_files_unlocked(snapshot, generation.clone())
            })?;
            self.renew_validation_receipt_unlocked(snapshot, Some(&generation))
        })
    }

    fn renew_validation_receipt_unlocked(
        &self,
        expected: &IssueSnapshot,
        generation: Option<&CacheGeneration>,
    ) -> Result<ValidationReceiptRenewal, CacheError> {
        if self.current_generation_unlocked(expected.number)?.as_ref() != generation {
            return Ok(ValidationReceiptRenewal::GenerationChanged);
        }
        let Some(current) = self.load_entry(expected.number) else {
            return Ok(ValidationReceiptRenewal::EntryUnreadable);
        };
        if !persisted_snapshots_match(&current.snapshot, expected) {
            return Ok(ValidationReceiptRenewal::SnapshotMismatch);
        }
        let Some(generation) = generation else {
            return Ok(ValidationReceiptRenewal::GenerationMissing);
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
        Ok(ValidationReceiptRenewal::Renewed)
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
        let body = read_payload_string(&dir.join("body.md")).ok()?;
        let meta_bytes = read_payload(&dir.join("meta.json")).ok()?;
        let meta: CacheMeta = serde_json::from_slice(&meta_bytes).ok()?;

        // Re-hydrate comment snapshots from the comments/ directory.
        let mut comments: Vec<CommentSnapshot> = Vec::new();
        let comments_dir = dir.join("comments");
        if comments_dir.is_dir() {
            for cid in &meta.comment_ids {
                let path = comments_dir.join(format!("{cid}.md"));
                if let Ok(body) = read_payload_string(&path) {
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
            author_login: meta.author_login.clone(),
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
        // Plain Issue (no `<!-- gwt-spec id=... -->` header at all) and a
        // body whose SPEC structure cannot be parsed (malformed sections
        // index, missing referenced comment, a header quoted in prose) both
        // surface with an empty SpecBody. The malformed case also carries
        // the parse error: hiding it made every validated read fail forever
        // (Issue #4392), and `SpecOps::write_section` refuses it so the empty
        // section map never rewrites the index and orphans comment content.
        let (spec_body, spec_parse_error) = match SpecBody::parse(&snapshot.body, &parsed_comments)
        {
            Ok(spec_body) => (spec_body, None),
            Err(error) => {
                let spec_parse_error = match error {
                    ParseError::MissingHeader => None,
                    error => Some(error.to_string()),
                };
                let empty = SpecBody {
                    meta: SpecMeta {
                        id: meta.number.to_string(),
                        version: 1,
                    },
                    sections_index: crate::body::SectionsIndex::default(),
                    sections: std::collections::BTreeMap::new(),
                };
                (empty, spec_parse_error)
            }
        };
        Some(CacheEntry {
            snapshot,
            spec_body,
            spec_parse_error,
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

    /// List readable Issue fields without opening comments or parsing SPECs.
    /// Unchanged roots reuse one shared, bounded cache across `Cache` instances;
    /// warm reads inspect only root and writer-stamp metadata.
    pub fn list_summaries(&self) -> Result<Arc<Vec<IssueSummary>>, CacheError> {
        let Some(key) = self.summary_cache_key()? else {
            return Ok(Arc::new(Vec::new()));
        };
        {
            let cached = summary_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(entry) = cached
                .iter()
                .find(|entry| entry.root == self.root && entry.key == key)
            {
                return Ok(Arc::clone(&entry.summaries));
            }
        }

        let mut summaries = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir()
                || entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<u64>().ok())
                    .is_none()
            {
                continue;
            }
            let directory = entry.path();
            let Ok(body) = read_payload_string(&directory.join("body.md")) else {
                continue;
            };
            let Ok(meta_bytes) = read_payload(&directory.join("meta.json")) else {
                continue;
            };
            let Ok(meta) = serde_json::from_slice::<CacheMeta>(&meta_bytes) else {
                continue;
            };
            summaries.push(IssueSummary {
                number: meta.number,
                title: meta.title,
                body,
                labels: meta.labels,
                state: if meta.state == "closed" {
                    IssueState::Closed
                } else {
                    IssueState::Open
                },
                updated_at: meta.updated_at,
            });
        }
        let summaries = Arc::new(summaries);
        // A refresh overlapping the scan must not make that scan reusable.
        // Like list_entries, a single scan is not a multi-Issue transaction.
        if self.summary_cache_key()?.as_ref() == Some(&key) {
            let mut cached = summary_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            cached.retain(|entry| entry.root != self.root);
            if cached.len() >= SUMMARY_CACHE_ROOT_LIMIT {
                cached.remove(0);
            }
            cached.push(SummaryCacheEntry {
                root: self.root.clone(),
                key,
                summaries: Arc::clone(&summaries),
            });
        }
        Ok(summaries)
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

fn read_payload(path: &Path) -> std::io::Result<Vec<u8>> {
    let bytes = fs::read(path)?;
    #[cfg(test)]
    record_payload_read(path, bytes.len());
    Ok(bytes)
}

fn read_payload_string(path: &Path) -> std::io::Result<String> {
    let text = fs::read_to_string(path)?;
    #[cfg(test)]
    record_payload_read(path, text.len());
    Ok(text)
}

#[cfg(test)]
#[derive(Clone, Copy, Default)]
struct PayloadReads {
    bytes: usize,
    comment_bytes: usize,
}

#[cfg(test)]
std::thread_local! {
    static PAYLOAD_READS: std::cell::Cell<PayloadReads> = const {
        std::cell::Cell::new(PayloadReads { bytes: 0, comment_bytes: 0 })
    };
}

#[cfg(test)]
fn record_payload_read(path: &Path, bytes: usize) {
    PAYLOAD_READS.with(|counter| {
        let mut reads = counter.get();
        reads.bytes += bytes;
        if path.parent().and_then(Path::file_name) == Some(std::ffi::OsStr::new("comments")) {
            reads.comment_bytes += bytes;
        }
        counter.set(reads);
    });
}

#[cfg(test)]
fn take_payload_reads() -> PayloadReads {
    PAYLOAD_READS.with(std::cell::Cell::take)
}

fn persisted_snapshots_match(left: &IssueSnapshot, right: &IssueSnapshot) -> bool {
    left.number == right.number
        && left.author_login == right.author_login
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
    write_atomic_with_durability(path, bytes, Durability::FlushToDevice)
}

/// Whether an atomic write waits for the storage device before returning.
///
/// Issue #3777: `sync_all` is the only call in this helper that blocks on the
/// device, and on a contended Windows runner it measured 522ms for a
/// half-kilobyte file. A UserPromptSubmit hook performs several such writes
/// under a 200ms budget, so state that the next hook event rewrites anyway
/// asks for [`Durability::RenameOnly`]: the rename still publishes the file
/// whole, only the "survives an OS crash" guarantee is dropped.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Wait for the device. For data that must survive an OS crash.
    FlushToDevice,
    /// Publish atomically without waiting for the device. For transient state
    /// that is rewritten on the next event.
    RenameOnly,
}

#[doc(hidden)]
pub fn write_atomic_with_durability(
    path: &Path,
    bytes: &[u8],
    durability: Durability,
) -> std::io::Result<()> {
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
        if durability == Durability::FlushToDevice {
            f.sync_all()?;
        }
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

#[cfg(test)]
mod summary_tests {
    use super::*;
    use std::{sync::Arc, time::Instant};
    use tempfile::TempDir;

    fn fixture(root: &Path, number: u64, body: &str, comment: &str) {
        let directory = root.join(number.to_string());
        fs::create_dir_all(directory.join("comments")).unwrap();
        fs::write(directory.join("body.md"), body).unwrap();
        fs::write(directory.join("comments/1.md"), comment).unwrap();
        fs::write(
            directory.join("meta.json"),
            serde_json::to_vec(&CacheMeta {
                author_login: None,
                number,
                title: format!("Issue {number}"),
                labels: vec!["phase/ready".to_owned()],
                state: "open".to_owned(),
                updated_at: "2026-10-14T00:00:00Z".to_owned(),
                comment_ids: vec![1],
                generation: Some("fixture".to_owned()),
            })
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn summaries_share_warm_cache_without_payload_or_comment_reads() {
        let temp = TempDir::new().unwrap();
        fixture(temp.path(), 1, "summary body", "large comment");
        let cache = Cache::new(temp.path().to_path_buf());
        let _ = take_payload_reads();
        let cold = cache.list_summaries().unwrap();
        let reads = take_payload_reads();
        assert_eq!(cold[0].body, "summary body");
        assert_eq!(reads.comment_bytes, 0);
        assert!(reads.bytes > 0);

        let warm = Cache::new(temp.path().to_path_buf())
            .list_summaries()
            .unwrap();
        assert!(Arc::ptr_eq(&cold, &warm));
        assert_eq!(take_payload_reads().bytes, 0);
    }

    #[test]
    fn summaries_invalidate_for_every_canonical_writer_and_root_changes() {
        let temp = TempDir::new().unwrap();
        fixture(temp.path(), 1, "body", "comment");
        let reader = Cache::new(temp.path().to_path_buf());
        let writer = Cache::new(temp.path().to_path_buf());
        let first = reader.list_summaries().unwrap();
        writer
            .apply_phase_change(IssueNumber(1), vec!["phase/verify".to_owned()])
            .unwrap();
        assert_eq!(reader.list_summaries().unwrap()[0].labels, ["phase/verify"]);

        let mut snapshot = writer.load_entry(IssueNumber(1)).unwrap().snapshot;
        snapshot.title = "snapshot write".to_owned();
        writer.write_snapshot(&snapshot).unwrap();
        assert_eq!(reader.list_summaries().unwrap()[0].title, snapshot.title);
        let generation = writer.current_generation(IssueNumber(1)).unwrap();
        snapshot.title = "conditional write".to_owned();
        assert!(writer
            .write_snapshot_if_generation(&snapshot, generation.as_ref())
            .unwrap()
            .is_some());
        assert_eq!(reader.list_summaries().unwrap()[0].title, snapshot.title);
        snapshot.title = "receipt write".to_owned();
        writer.write_snapshot_with_receipt(&snapshot).unwrap();
        let updated = reader.list_summaries().unwrap();
        assert_eq!(updated[0].title, snapshot.title);
        assert!(!Arc::ptr_eq(&first, &updated));

        fixture(temp.path(), 2, "new issue", "comment");
        assert_eq!(reader.list_summaries().unwrap().len(), 2);
        fs::remove_dir_all(temp.path().join("2")).unwrap();
        assert_eq!(reader.list_summaries().unwrap().len(), 1);

        let other = TempDir::new().unwrap();
        fixture(other.path(), 1, "other root", "comment");
        assert_eq!(
            Cache::new(other.path().to_path_buf())
                .list_summaries()
                .unwrap()[0]
                .body,
            "other root"
        );
        assert_eq!(reader.list_summaries().unwrap()[0].body, "body");
    }

    #[test]
    fn summaries_skip_malformed_entries_and_absent_roots() {
        let temp = TempDir::new().unwrap();
        let cache = Cache::new(temp.path().join("absent"));
        assert!(cache.list_summaries().unwrap().is_empty());
        fixture(temp.path(), 1, "readable", "comment");
        fixture(temp.path(), 2, "bad", "comment");
        fs::write(temp.path().join("2/meta.json"), "broken").unwrap();
        let summaries = Cache::new(temp.path().to_path_buf())
            .list_summaries()
            .unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].number, 1);
    }

    #[test]
    fn summaries_observe_an_external_generation_stamp() {
        let temp = TempDir::new().unwrap();
        fixture(temp.path(), 1, "before external write", "comment");
        let cache = Cache::new(temp.path().to_path_buf());
        let before = cache.list_summaries().unwrap();

        // An independent process cannot evict this process's memory cache.
        // Publish its disk change directly, including the writer stamp.
        fs::write(temp.path().join("1/body.md"), "after external write").unwrap();
        write_atomic_with_durability(
            &temp.path().join(SUMMARY_GENERATION_FILE),
            Uuid::new_v4().to_string().as_bytes(),
            Durability::RenameOnly,
        )
        .unwrap();
        let after = cache.list_summaries().unwrap();
        assert!(!Arc::ptr_eq(&before, &after));
        assert_eq!(after[0].body, "after external write");
    }

    #[test]
    fn summaries_1500_issue_read_cost() {
        let temp = TempDir::new().unwrap();
        let body = "body content\n".repeat(160);
        let comment = "comment content\n".repeat(1024);
        for number in 1..=1500 {
            fixture(temp.path(), number, &body, &comment);
        }
        let cache = Cache::new(temp.path().to_path_buf());
        // Compare repeated reads on both sides, after the old path has also
        // warmed the operating system's filesystem cache.
        assert_eq!(cache.list_entries().unwrap().len(), 1500);
        let _ = take_payload_reads();
        let started = Instant::now();
        let old = cache.list_entries().unwrap();
        let before_time = started.elapsed();
        let before_reads = take_payload_reads();
        let started = Instant::now();
        let cold = cache.list_summaries().unwrap();
        let cold_time = started.elapsed();
        let cold_reads = take_payload_reads();
        let started = Instant::now();
        let warm = cache.list_summaries().unwrap();
        let warm_time = started.elapsed();
        let warm_reads = take_payload_reads();
        assert_eq!(old.len(), 1500);
        assert_eq!(cold.len(), old.len());
        assert!(Arc::ptr_eq(&cold, &warm));
        assert_eq!(before_reads.comment_bytes, comment.len() * 1500);
        assert_eq!(cold_reads.comment_bytes, 0);
        assert!(cold_reads.bytes < before_reads.bytes);
        assert_eq!(warm_reads.bytes, 0);
        println!(
            "1500 issues: before={}us/{}bytes, cold={}us/{}bytes, warm={}us/{}bytes",
            before_time.as_micros(),
            before_reads.bytes,
            cold_time.as_micros(),
            cold_reads.bytes,
            warm_time.as_micros(),
            warm_reads.bytes,
        );
    }
}
