//! Artifact Operability Record (SPEC-3248 P7C, T-274 core).
//!
//! Oversized owners (like SPEC #3248 itself) route sections through the
//! supported multipart writer. This module persists the machine-readable
//! ledger of those writes per owner: for every section, the byte size,
//! resident location, comment ids/parts, content hash, largest part
//! payload, readback status, and write timestamp. Downstream consumers —
//! the Phase Launch Packet (T-275) and stale-hash rejection (T-279) — read
//! this record instead of re-deriving artifact shape from the GitHub
//! snapshot.
//!
//! The record is machine-local, repo-scoped state:
//! `~/.gwt/projects/<repo-hash>/operability/issue-<owner>.json`. Writes are
//! best-effort from the `issue.spec.edit` success path (a ledger failure
//! must not fail a verified write); unresolvable repo hashes (non-git dirs)
//! skip persistence.
//!
//! Follow-ups (T-274 full): phase-slice binding, payload/token budgets,
//! read/write strategy fields, and owner-revision binding for T-279.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Per-section operability facts from one verified write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionOperability {
    pub bytes: usize,
    /// Comment parts written (0 = body-resident).
    pub parts: usize,
    /// `"body"` or `"comments"`.
    pub location: String,
    /// Comment ids holding the section, in part order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comment_ids: Vec<u64>,
    /// sha256 of the canonical section content.
    pub sha256: String,
    pub largest_part_bytes: usize,
    /// The write path readback-verified the content (the multipart writer
    /// rolls back unverified writes, so persisted records are verified).
    pub readback_verified: bool,
    pub written_at: DateTime<Utc>,
}

/// The per-owner operability ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactOperabilityRecord {
    pub owner_number: u64,
    pub sections: BTreeMap<String, SectionOperability>,
    pub updated_at: DateTime<Utc>,
}

/// Resolve the record path. `None` when the repo hash is unresolvable.
#[must_use]
pub fn record_path(repo_path: &Path, owner_number: u64) -> Option<PathBuf> {
    let repo_hash = crate::index_worker::detect_repo_hash(repo_path)?;
    Some(
        gwt_core::paths::gwt_projects_dir()
            .join(repo_hash.as_str())
            .join("operability")
            .join(format!("issue-{owner_number}.json")),
    )
}

/// Load the owner's record. `Ok(None)` when absent or in degenerate mode.
pub fn load(repo_path: &Path, owner_number: u64) -> io::Result<Option<ArtifactOperabilityRecord>> {
    let Some(path) = record_path(repo_path, owner_number) else {
        return Ok(None);
    };
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let record = serde_json::from_str::<ArtifactOperabilityRecord>(&contents)
                .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
            Ok(Some(record))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Merge one verified write receipt into the owner's ledger. Best-effort
/// no-op in degenerate mode.
pub fn record_write(
    repo_path: &Path,
    owner_number: u64,
    section: &str,
    receipt: &gwt_github::WriteReceipt,
) -> io::Result<()> {
    let Some(path) = record_path(repo_path, owner_number) else {
        return Ok(());
    };
    let mut record = match load(repo_path, owner_number) {
        Ok(Some(record)) => record,
        // Absent or unreadable ledgers restart cleanly — the ledger mirrors
        // verified remote writes, so rebuilding from now-on is safe.
        _ => ArtifactOperabilityRecord {
            owner_number,
            sections: BTreeMap::new(),
            updated_at: Utc::now(),
        },
    };
    record.sections.insert(
        section.to_string(),
        SectionOperability {
            bytes: receipt.bytes,
            parts: receipt.parts,
            location: receipt.location.clone(),
            comment_ids: receipt.comment_ids.clone(),
            sha256: receipt.sha256.clone(),
            largest_part_bytes: receipt.largest_part_bytes,
            readback_verified: true,
            written_at: Utc::now(),
        },
    );
    record.updated_at = Utc::now();
    let serialized = serde_json::to_vec_pretty(&record)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    gwt_github::cache::write_atomic(&path, &serialized)
}

/// Best-effort wrapper for the `issue.spec.edit` success path: a ledger
/// failure is logged, never surfaced (the remote write is already verified).
pub fn record_write_best_effort(
    repo_path: &Path,
    owner_number: u64,
    section: &str,
    receipt: &gwt_github::WriteReceipt,
) {
    if let Err(error) = record_write(repo_path, owner_number, section, receipt) {
        tracing::warn!(
            ?error,
            owner_number,
            section,
            "artifact operability ledger update failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedEnvVar;

    fn receipt(bytes: usize, parts: usize) -> gwt_github::WriteReceipt {
        gwt_github::WriteReceipt {
            bytes,
            parts,
            sha256: format!("hash-{bytes}"),
            location: if parts == 0 { "body" } else { "comments" }.to_string(),
            comment_ids: (0..parts as u64).map(|i| 100 + i).collect(),
            largest_part_bytes: if parts <= 1 { bytes } else { bytes / parts + 1 },
        }
    }

    // T-274: verified writes accumulate into one per-owner ledger keyed by
    // section, replacing the section entry on rewrite.
    #[test]
    fn record_write_accumulates_per_owner_ledger() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let dir = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(dir.path());

        record_write(dir.path(), 3248, "tasks", &receipt(90_000, 2)).unwrap();
        record_write(dir.path(), 3248, "spec", &receipt(4_000, 0)).unwrap();
        let record = load(dir.path(), 3248).unwrap().unwrap();
        assert_eq!(record.owner_number, 3248);
        assert_eq!(record.sections.len(), 2);
        let tasks = &record.sections["tasks"];
        assert_eq!(tasks.parts, 2);
        assert_eq!(tasks.location, "comments");
        assert_eq!(tasks.comment_ids, vec![100, 101]);
        assert!(tasks.readback_verified);
        assert_eq!(record.sections["spec"].location, "body");

        // Rewriting a section replaces its entry.
        record_write(dir.path(), 3248, "tasks", &receipt(95_000, 3)).unwrap();
        let record = load(dir.path(), 3248).unwrap().unwrap();
        assert_eq!(record.sections.len(), 2);
        assert_eq!(record.sections["tasks"].parts, 3);
        assert_eq!(record.sections["tasks"].sha256, "hash-95000");

        // Owners do not share ledgers.
        assert_eq!(load(dir.path(), 9999).unwrap(), None);
        // The ledger lives under HOME, not the worktree.
        let path = record_path(dir.path(), 3248).unwrap();
        assert!(path.starts_with(home.path()));
    }

    // Degenerate mode (non-git dir): persistence is skipped, loads are None.
    #[test]
    fn non_git_dir_is_degenerate_noop() {
        let dir = tempfile::tempdir().unwrap();
        assert!(record_path(dir.path(), 1).is_none());
        record_write(dir.path(), 1, "tasks", &receipt(10, 0)).unwrap();
        assert_eq!(load(dir.path(), 1).unwrap(), None);
    }
}
