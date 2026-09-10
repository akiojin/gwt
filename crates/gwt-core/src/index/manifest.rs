//! `manifest.json` schema and helpers for incremental indexing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{GwtError, Result};

pub const SCHEMA_VERSION: u32 = 1;

/// One entry in `manifest-{scope}.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub mtime: i64,
    pub size: u64,
}

/// Persisted manifest payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub scope: String,
    pub entries: Vec<ManifestEntry>,
}

/// Generation-local Phase 71 manifest entry. Unlike the mutable legacy
/// manifest above, identity is content-addressed and independent of mtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIndexV2ManifestEntry {
    pub path: String,
    pub source_object: String,
    pub source_digest: String,
    pub payload_digest: String,
    pub metadata_digest: String,
    pub scope: String,
    pub cas_key: String,
    pub input_digest: String,
    pub vector_checksum: String,
    pub dimension: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIndexV2Manifest {
    pub schema_version: u32,
    pub entries: Vec<FileIndexV2ManifestEntry>,
}

impl Manifest {
    pub fn new(scope: impl Into<String>, entries: Vec<ManifestEntry>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            scope: scope.into(),
            entries,
        }
    }
}

fn manifest_path(worktree_dir: &Path, scope: &str) -> PathBuf {
    // The manifest sits in the worktree-level directory. Accept either the
    // worktree-level dir or a scope-leaf dir (e.g. `.../files`); both are
    // normalized to the worktree level so writers and readers always agree.
    let leaf = worktree_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let base = if matches!(leaf, "specs" | "files" | "files-docs" | "issues") {
        worktree_dir.parent().unwrap_or(worktree_dir)
    } else {
        worktree_dir
    };
    base.join(format!("manifest-{scope}.json"))
}

pub fn read_manifest(db_dir: &Path, scope: &str) -> Result<Vec<ManifestEntry>> {
    let path = manifest_path(db_dir, scope);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&path)
        .map_err(|e| GwtError::Other(format!("read manifest {}: {e}", path.display())))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| GwtError::Other(format!("parse manifest {}: {e}", path.display())))?;
    Ok(manifest.entries)
}

pub fn write_manifest(db_dir: &Path, scope: &str, entries: Vec<ManifestEntry>) -> Result<()> {
    let path = manifest_path(db_dir, scope);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manifest = Manifest::new(scope, entries);
    let payload = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| GwtError::Other(format!("serialize manifest: {e}")))?;
    std::fs::write(&path, payload)
        .map_err(|e| GwtError::Other(format!("write manifest {}: {e}", path.display())))?;
    Ok(())
}

#[derive(Debug, Default)]
pub struct ManifestDiff {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

pub fn compute_manifest_diff(old: &[ManifestEntry], new: &[ManifestEntry]) -> ManifestDiff {
    use std::collections::HashMap;
    let old_map: HashMap<&str, &ManifestEntry> = old.iter().map(|e| (e.path.as_str(), e)).collect();
    let new_map: HashMap<&str, &ManifestEntry> = new.iter().map(|e| (e.path.as_str(), e)).collect();

    let mut diff = ManifestDiff::default();
    for (path, entry) in &new_map {
        match old_map.get(path) {
            None => diff.added.push((*path).to_string()),
            Some(old_entry) => {
                if old_entry.mtime != entry.mtime || old_entry.size != entry.size {
                    diff.changed.push((*path).to_string());
                }
            }
        }
    }
    for path in old_map.keys() {
        if !new_map.contains_key(path) {
            diff.removed.push((*path).to_string());
        }
    }
    diff.added.sort();
    diff.changed.sort();
    diff.removed.sort();
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, mtime: i64, size: u64) -> ManifestEntry {
        ManifestEntry {
            path: path.into(),
            mtime,
            size,
        }
    }

    #[test]
    fn round_trip_persists_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("worktrees/wt/files");
        std::fs::create_dir_all(&db).unwrap();
        let entries = vec![entry("a", 1, 10), entry("b", 2, 20)];
        write_manifest(&db, "files", entries.clone()).unwrap();
        let loaded = read_manifest(&db, "files").unwrap();
        assert_eq!(loaded, entries);
    }

    #[test]
    fn read_returns_empty_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = read_manifest(tmp.path(), "files").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn diff_detects_added_changed_removed() {
        let old = vec![entry("a", 1, 10), entry("b", 1, 20)];
        let new = vec![entry("a", 2, 10), entry("c", 1, 30)];
        let diff = compute_manifest_diff(&old, &new);
        assert_eq!(diff.added, vec!["c"]);
        assert_eq!(diff.changed, vec!["a"]);
        assert_eq!(diff.removed, vec!["b"]);
    }

    #[test]
    fn file_index_v2_manifest_roundtrips_without_legacy_mtime_identity() {
        let manifest = FileIndexV2Manifest {
            schema_version: 1,
            entries: vec![FileIndexV2ManifestEntry {
                path: "src/lib.rs".into(),
                source_object: "git:0123".into(),
                source_digest: "source".into(),
                payload_digest: "payload".into(),
                metadata_digest: "metadata".into(),
                scope: "files".into(),
                cas_key: "cas".into(),
                input_digest: "input".into(),
                vector_checksum: "vector".into(),
                dimension: 768,
            }],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        assert_eq!(
            serde_json::from_str::<FileIndexV2Manifest>(&json).unwrap(),
            manifest
        );
        assert!(!json.contains("mtime"));
    }
}
