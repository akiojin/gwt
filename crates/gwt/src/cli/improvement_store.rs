use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use fs2::FileExt;
use gwt_github::{cache::write_atomic, client::ApiError, SpecOpsError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::improvement::{CandidateStore, ImprovementCandidate};

pub(super) const STORE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct LegacyImportState {
    #[serde(default)]
    sources: Vec<LegacySourceRecord>,
    #[serde(default)]
    diagnostics: Vec<LegacyDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacySourceRecord {
    source_id: String,
    content_digest: String,
    imported_candidates: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyDiagnostic {
    source_id: String,
    code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LegacyProvenance {
    pub(super) source_id: String,
    pub(super) content_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ResolutionAttemptLease {
    pub(super) attempt_id: String,
    pub(super) lease_owner: String,
    pub(super) started_at: chrono::DateTime<chrono::Utc>,
    pub(super) expires_at: chrono::DateTime<chrono::Utc>,
    pub(super) remote_phase: AttemptRemotePhase,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum AttemptRemotePhase {
    NotSubmitted,
    Submitted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AttemptLeaseDecision {
    Acquired(ResolutionAttemptLease),
    Busy {
        attempt_id: String,
        lease_owner: String,
        expires_at: chrono::DateTime<chrono::Utc>,
    },
    RemoteOutcomeUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyRootKind {
    Worktree,
    Session,
}

#[derive(Debug, Clone)]
struct LegacyRoot {
    path: PathBuf,
    kind: LegacyRootKind,
}

pub(super) fn candidate_store_path(repo_root: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_root)
        .join("improvements")
        .join("candidates.json")
}

pub(super) fn evidence_dir_path(repo_root: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_root)
        .join("improvements")
        .join("evidence")
}

pub(super) fn load_and_repair(repo_root: &Path) -> Result<CandidateStore, SpecOpsError> {
    with_store_lock(repo_root, || load_and_repair_unlocked(repo_root))
}

pub(super) fn update<T>(
    repo_root: &Path,
    operation: impl FnOnce(&mut CandidateStore) -> Result<T, SpecOpsError>,
) -> Result<T, SpecOpsError> {
    with_store_lock(repo_root, || {
        let mut store = load_and_repair_unlocked(repo_root)?;
        let result = operation(&mut store)?;
        save_unlocked(repo_root, &store)?;
        Ok(result)
    })
}

pub(super) fn acquire_attempt_lease(
    store: &mut CandidateStore,
    candidate_id: &str,
    lease_owner: &str,
    now: chrono::DateTime<chrono::Utc>,
    ttl: chrono::Duration,
) -> Result<AttemptLeaseDecision, SpecOpsError> {
    let candidate = store
        .candidates
        .iter_mut()
        .find(|candidate| candidate.id == candidate_id)
        .ok_or_else(|| invalid("candidate not found"))?;
    if let Some(current) = candidate.attempt.as_ref() {
        if current.expires_at > now {
            return Ok(AttemptLeaseDecision::Busy {
                attempt_id: current.attempt_id.clone(),
                lease_owner: current.lease_owner.clone(),
                expires_at: current.expires_at,
            });
        }
        if current.remote_phase == AttemptRemotePhase::Submitted {
            candidate.state = "remote-outcome-unknown".to_string();
            candidate.updated_at = now.to_rfc3339();
            return Ok(AttemptLeaseDecision::RemoteOutcomeUnknown);
        }
    }
    let lease = ResolutionAttemptLease {
        attempt_id: format!("attempt-{}", Uuid::new_v4().simple()),
        lease_owner: lease_owner.to_string(),
        started_at: now,
        expires_at: now + ttl,
        remote_phase: AttemptRemotePhase::NotSubmitted,
    };
    candidate.attempt = Some(lease.clone());
    candidate.updated_at = now.to_rfc3339();
    Ok(AttemptLeaseDecision::Acquired(lease))
}

pub(super) fn mark_attempt_submitted(
    store: &mut CandidateStore,
    candidate_id: &str,
    attempt_id: &str,
) -> Result<(), SpecOpsError> {
    let candidate = store
        .candidates
        .iter_mut()
        .find(|candidate| candidate.id == candidate_id)
        .ok_or_else(|| invalid("candidate not found"))?;
    let attempt = candidate
        .attempt
        .as_mut()
        .filter(|attempt| attempt.attempt_id == attempt_id)
        .ok_or_else(|| invalid("promotion attempt lease is stale"))?;
    attempt.remote_phase = AttemptRemotePhase::Submitted;
    Ok(())
}

fn load_and_repair_unlocked(repo_root: &Path) -> Result<CandidateStore, SpecOpsError> {
    let canonical_path = candidate_store_path(repo_root);
    let mut store = if canonical_path.exists() {
        let raw = fs::read_to_string(&canonical_path).map_err(io_as_spec_error)?;
        serde_json::from_str(&raw).map_err(serde_as_spec_error)?
    } else {
        CandidateStore {
            schema_version: STORE_SCHEMA_VERSION,
            candidates: Vec::new(),
            legacy_import: LegacyImportState::default(),
        }
    };
    if store.schema_version > STORE_SCHEMA_VERSION {
        return Err(invalid(&format!(
            "unsupported improvement store schema version: {}",
            store.schema_version
        )));
    }

    let mut changed = store.schema_version != STORE_SCHEMA_VERSION;
    store.schema_version = STORE_SCHEMA_VERSION;
    let mut seen_roots = HashSet::new();
    for root in discover_legacy_roots(repo_root) {
        let source_id = digest_bytes(normalized_source_identity(&root.path).as_bytes());
        let canonical_root = match fs::canonicalize(&root.path) {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if root.kind == LegacyRootKind::Session {
                    changed |=
                        push_diagnostic(&mut store.legacy_import, &source_id, "missing-source");
                }
                continue;
            }
            Err(error) => {
                let code = if error.kind() == io::ErrorKind::PermissionDenied {
                    "permission-denied"
                } else {
                    "source-unavailable"
                };
                changed |= push_diagnostic(&mut store.legacy_import, &source_id, code);
                continue;
            }
        };
        if !seen_roots.insert(canonical_root) {
            changed |= push_diagnostic(&mut store.legacy_import, &source_id, "alias-source");
            continue;
        }
        changed |= import_legacy_source(&root, &canonical_path, &source_id, repo_root, &mut store)?;
    }
    if changed || (!canonical_path.exists() && !store.candidates.is_empty()) {
        save_unlocked(repo_root, &store)?;
    }
    Ok(store)
}

fn save_unlocked(repo_root: &Path, store: &CandidateStore) -> Result<(), SpecOpsError> {
    let path = candidate_store_path(repo_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_as_spec_error)?;
    }
    let mut persisted = store.clone();
    persisted.schema_version = STORE_SCHEMA_VERSION;
    let bytes = serde_json::to_vec_pretty(&persisted).map_err(serde_as_spec_error)?;
    write_atomic_and_sync_parent(&path, &bytes).map_err(io_as_spec_error)
}

fn with_store_lock<T>(
    repo_root: &Path,
    operation: impl FnOnce() -> Result<T, SpecOpsError>,
) -> Result<T, SpecOpsError> {
    let improvements_dir =
        gwt_core::paths::gwt_project_dir_for_repo_path(repo_root).join("improvements");
    fs::create_dir_all(&improvements_dir).map_err(io_as_spec_error)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(improvements_dir.join(".lock"))
        .map_err(io_as_spec_error)?;
    lock.lock_exclusive().map_err(io_as_spec_error)?;
    let result = operation();
    let unlock_result = FileExt::unlock(&lock).map_err(io_as_spec_error);
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn discover_legacy_roots(repo_root: &Path) -> Vec<LegacyRoot> {
    let mut roots = vec![LegacyRoot {
        path: repo_root.to_path_buf(),
        kind: LegacyRootKind::Worktree,
    }];
    if let Ok(main_root) = gwt_git::worktree::main_worktree_root(repo_root) {
        if let Ok(worktrees) = gwt_git::WorktreeManager::new(main_root).list() {
            roots.extend(worktrees.into_iter().map(|worktree| LegacyRoot {
                path: worktree.path,
                kind: LegacyRootKind::Worktree,
            }));
        }
    }
    let repo_hash = gwt_core::paths::project_scope_hash(repo_root);
    if let Ok(entries) = fs::read_dir(gwt_core::paths::gwt_sessions_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
                continue;
            }
            let Ok(session) = gwt_agent::Session::load(&path) else {
                continue;
            };
            let belongs_to_repo = session.repo_hash.as_deref() == Some(repo_hash.as_str())
                || (session.worktree_path.exists()
                    && gwt_core::repo_hash::detect_repo_hash(&session.worktree_path).as_ref()
                        == Some(&repo_hash));
            if belongs_to_repo {
                roots.push(LegacyRoot {
                    path: session.worktree_path,
                    kind: LegacyRootKind::Session,
                });
            }
        }
    }
    roots.sort_by(|left, right| left.path.cmp(&right.path));
    roots.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
    roots
}

fn import_legacy_source(
    root: &LegacyRoot,
    canonical_path: &Path,
    source_id: &str,
    repo_root: &Path,
    store: &mut CandidateStore,
) -> Result<bool, SpecOpsError> {
    let source_path = root
        .path
        .join(".gwt")
        .join("improvements")
        .join("candidates.json");
    if source_path == canonical_path {
        return Ok(false);
    }
    let metadata = match fs::symlink_metadata(&source_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if root.kind == LegacyRootKind::Session {
                return Ok(push_diagnostic(
                    &mut store.legacy_import,
                    source_id,
                    "missing-source",
                ));
            }
            return Ok(false);
        }
        Err(error) => return Err(io_as_spec_error(error)),
    };
    if metadata.file_type().is_symlink() {
        return Ok(push_diagnostic(
            &mut store.legacy_import,
            source_id,
            "symlink-source",
        ));
    }
    let bytes = match fs::read(&source_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            let code = if error.kind() == io::ErrorKind::PermissionDenied {
                "permission-denied"
            } else {
                "read-failed"
            };
            return Ok(push_diagnostic(&mut store.legacy_import, source_id, code));
        }
    };
    let content_digest = digest_bytes(&bytes);
    if store
        .legacy_import
        .sources
        .iter()
        .any(|source| source.source_id == source_id && source.content_digest == content_digest)
    {
        return Ok(false);
    }
    let legacy: CandidateStore = match serde_json::from_slice(&bytes) {
        Ok(store) => store,
        Err(_) => {
            return Ok(push_diagnostic(
                &mut store.legacy_import,
                source_id,
                "malformed-json",
            ));
        }
    };
    let mut imported = 0usize;
    for mut candidate in legacy.candidates {
        migrate_legacy_evidence(
            repo_root,
            source_id,
            &mut candidate,
            &mut store.legacy_import,
        )?;
        let fingerprint = legacy_fingerprint(&candidate);
        let aggregate = candidate
            .legacy_occurrence_count
            .unwrap_or(candidate.occurrences);
        if let Some(existing) = store
            .candidates
            .iter_mut()
            .find(|existing| existing.fingerprint.as_deref() == Some(fingerprint.as_str()))
        {
            if let Some(provenance) = existing
                .legacy_provenance
                .iter_mut()
                .find(|provenance| provenance.source_id == source_id)
            {
                existing.legacy_occurrence_count = Some(
                    existing
                        .legacy_occurrence_count
                        .unwrap_or_default()
                        .max(aggregate),
                );
                provenance.content_digest = content_digest.clone();
            } else {
                existing.legacy_occurrence_count = Some(
                    existing
                        .legacy_occurrence_count
                        .unwrap_or_default()
                        .saturating_add(aggregate),
                );
                existing.legacy_provenance.push(LegacyProvenance {
                    source_id: source_id.to_string(),
                    content_digest: content_digest.clone(),
                });
            }
        } else {
            candidate.schema_version = STORE_SCHEMA_VERSION;
            candidate.state = "needs-evidence".to_string();
            candidate.occurrences = 0;
            candidate.legacy_occurrence_count = Some(aggregate);
            candidate.fingerprint = Some(fingerprint);
            candidate.legacy_provenance = vec![LegacyProvenance {
                source_id: source_id.to_string(),
                content_digest: content_digest.clone(),
            }];
            store.candidates.push(candidate);
        }
        imported += 1;
    }
    store
        .legacy_import
        .sources
        .retain(|source| source.source_id != source_id);
    store.legacy_import.sources.push(LegacySourceRecord {
        source_id: source_id.to_string(),
        content_digest,
        imported_candidates: imported,
    });
    Ok(true)
}

fn migrate_legacy_evidence(
    repo_root: &Path,
    source_id: &str,
    candidate: &mut ImprovementCandidate,
    state: &mut LegacyImportState,
) -> Result<(), SpecOpsError> {
    for reference in &mut candidate.local_evidence {
        let Some(raw_path) = reference.path.take() else {
            continue;
        };
        if raw_path.trim().is_empty() || raw_path.contains("[redacted-path]") {
            push_diagnostic(state, source_id, "evidence-unavailable");
            continue;
        }
        let path = PathBuf::from(&raw_path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                push_diagnostic(state, source_id, "evidence-missing");
                continue;
            }
            Err(error) => {
                let code = if error.kind() == io::ErrorKind::PermissionDenied {
                    "evidence-permission-denied"
                } else {
                    "evidence-read-failed"
                };
                push_diagnostic(state, source_id, code);
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            push_diagnostic(state, source_id, "evidence-symlink");
            continue;
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                let code = if error.kind() == io::ErrorKind::PermissionDenied {
                    "evidence-permission-denied"
                } else {
                    "evidence-read-failed"
                };
                push_diagnostic(state, source_id, code);
                continue;
            }
        };
        let digest = digest_bytes(&bytes);
        let destination = evidence_dir_path(repo_root).join(&digest);
        if !destination.exists() {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(io_as_spec_error)?;
            }
            write_atomic_and_sync_parent(&destination, &bytes).map_err(io_as_spec_error)?;
        }
        reference.digest = Some(digest);
    }
    Ok(())
}

fn normalized_source_identity(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

fn legacy_fingerprint(candidate: &ImprovementCandidate) -> String {
    let identity = format!(
        "{}\0{}\0{}",
        candidate.dedupe_key, candidate.classification, candidate.target_artifact
    );
    format!("legacy:{}", digest_bytes(identity.as_bytes()))
}

fn digest_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn write_atomic_and_sync_parent(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic(path, bytes)?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn push_diagnostic(state: &mut LegacyImportState, source_id: &str, code: &str) -> bool {
    if state
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.source_id == source_id && diagnostic.code == code)
    {
        return false;
    }
    state.diagnostics.push(LegacyDiagnostic {
        source_id: source_id.to_string(),
        code: code.to_string(),
    });
    true
}

fn invalid(message: &str) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(message.to_string()))
}

fn io_as_spec_error(error: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(error.to_string()))
}

fn serde_as_spec_error(error: serde_json::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(error.to_string()))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};
    use serde_json::json;

    use super::*;

    fn store_with_candidate() -> CandidateStore {
        let candidate = serde_json::from_value(json!({
            "schema_version": 1,
            "id": "impr-lease",
            "created_at": "2026-07-13T00:00:00Z",
            "updated_at": "2026-07-13T00:00:00Z",
            "source": "agent-failure",
            "target_artifact": "skill",
            "classification": "gwt-caused",
            "confidence": "high",
            "state": "owner-resolving",
            "dedupe_key": "lease:test",
            "occurrences": 1,
            "sanitized_summary": "Lease test",
            "sanitized_details": null,
            "evidence_digest": null,
            "local_evidence": [],
            "linked_issue": null,
            "dismissed_reason": null
        }))
        .expect("candidate");
        CandidateStore {
            schema_version: STORE_SCHEMA_VERSION,
            candidates: vec![candidate],
            legacy_import: LegacyImportState::default(),
        }
    }

    #[test]
    fn attempt_lease_blocks_live_owner_and_takes_over_expired_not_submitted_attempt() {
        let mut store = store_with_candidate();
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 0, 0, 0).unwrap();
        let first = acquire_attempt_lease(
            &mut store,
            "impr-lease",
            "worker-a",
            now,
            Duration::seconds(30),
        )
        .expect("first lease");
        let first_id = match first {
            AttemptLeaseDecision::Acquired(lease) => lease.attempt_id,
            other => panic!("expected acquired lease, got {other:?}"),
        };

        assert!(matches!(
            acquire_attempt_lease(
                &mut store,
                "impr-lease",
                "worker-b",
                now + Duration::seconds(1),
                Duration::seconds(30),
            )
            .expect("busy lease"),
            AttemptLeaseDecision::Busy { .. }
        ));

        let takeover = acquire_attempt_lease(
            &mut store,
            "impr-lease",
            "worker-b",
            now + Duration::seconds(31),
            Duration::seconds(30),
        )
        .expect("takeover lease");
        let takeover_id = match takeover {
            AttemptLeaseDecision::Acquired(lease) => lease.attempt_id,
            other => panic!("expected takeover, got {other:?}"),
        };
        assert_ne!(takeover_id, first_id);
    }

    #[test]
    fn expired_submitted_attempt_becomes_remote_outcome_unknown_without_takeover() {
        let mut store = store_with_candidate();
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 0, 0, 0).unwrap();
        let first = acquire_attempt_lease(
            &mut store,
            "impr-lease",
            "worker-a",
            now,
            Duration::seconds(30),
        )
        .expect("first lease");
        let attempt_id = match first {
            AttemptLeaseDecision::Acquired(lease) => lease.attempt_id,
            other => panic!("expected acquired lease, got {other:?}"),
        };
        mark_attempt_submitted(&mut store, "impr-lease", &attempt_id).expect("mark submitted");

        assert!(matches!(
            acquire_attempt_lease(
                &mut store,
                "impr-lease",
                "worker-b",
                now + Duration::seconds(31),
                Duration::seconds(30),
            )
            .expect("unknown outcome"),
            AttemptLeaseDecision::RemoteOutcomeUnknown
        ));
        assert_eq!(store.candidates[0].state, "remote-outcome-unknown");
        assert_eq!(
            store.candidates[0]
                .attempt
                .as_ref()
                .expect("attempt")
                .attempt_id,
            attempt_id,
            "submitted attempt must not be replaced"
        );
    }
}
