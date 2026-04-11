//! Local cache for Issue snapshots (SPEC-12 FR-020〜FR-023).
//!
//! The cache is the source of truth for every UI consumer. All reads pass
//! through this layer without touching the network; writes happen only from
//! [`crate::client::IssueClient`] operations that explicitly flow through
//! `pull`-like commands.
//!
//! Filesystem layout (rooted at a configurable directory, typically
//! `~/.gwt/cache/issues/`):
//!
//! ```text
//! <root>/
//! └── <issue_number>/
//!     ├── body.md                  # verbatim Issue body
//!     ├── meta.json                # serialized CacheMeta
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

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::body::{ParseError, SpecBody, SpecMeta};
use crate::client::{
    CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt,
};
use crate::sections::SectionName;

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
}

impl CacheMeta {
    fn from_snapshot(snapshot: &IssueSnapshot) -> Self {
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
        }
    }
}

/// A loaded cache entry: the server snapshot plus a parsed [`SpecBody`] view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    pub snapshot: IssueSnapshot,
    pub spec_body: SpecBody,
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
                for (name, content) in spec_body.sections.iter() {
                    let filename = section_filename(name);
                    let path = sections_dir.join(&filename);
                    write_atomic(&path, content.as_bytes())?;
                    desired_sections.insert(filename);
                }
                prune_unlisted_files(&sections_dir, &desired_sections)?;
            }
            Err(ParseError::MissingHeader) => {
                // Plain GitHub Issues share the same cache root as SPEC Issues
                // but do not carry gwt-spec section markers. For those entries
                // we still cache body/meta/comments and simply omit `sections/`.
                let desired_sections: HashSet<String> = HashSet::new();
                prune_unlisted_files(&sections_dir, &desired_sections)?;
            }
            Err(err) => return Err(CacheError::Parse(err)),
        }

        // Finally, write meta.json.
        let meta = CacheMeta::from_snapshot(snapshot);
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
            body: body.clone(),
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
                meta: SpecMeta {
                    id: meta.number.to_string(),
                    version: 1,
                },
                sections_index: crate::body::SectionsIndex::default(),
                sections: std::collections::BTreeMap::new(),
            },
            Err(_) => return None,
        };
        Some(CacheEntry {
            snapshot,
            spec_body,
        })
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
}

/// Write bytes to `path` atomically via a `.tmp-<pid>-<nanos>` sibling file
/// followed by `rename`.
///
/// Exposed (via `cache::write_atomic`) so that other crates in the workspace
/// — notably `gwt-tui`'s hook handlers (SPEC #1942) — can reuse the exact
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
