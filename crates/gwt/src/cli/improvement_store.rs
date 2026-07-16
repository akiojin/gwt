use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use fs2::FileExt;
use gwt_github::{cache::write_atomic, client::ApiError, SpecOpsError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::improvement::{
    transition_candidate, validate_candidate_lifecycle, CandidateState, CandidateStore,
    ImprovementCandidate, ImprovementEligibility, RetryMetadata,
};
use super::improvement_contract::{
    OwnerProjectionAggregate, OwnerProjectionOccurrence, OwnerProjectionOwner,
    OwnerProjectionSnapshot, OWNER_PROJECTION_CONTRACT_REVISION,
};

pub(super) const STORE_SCHEMA_VERSION: u32 = 3;
const OWNER_PROJECTION_SCHEMA_VERSION: u32 = 2;
const LEGACY_OWNER_PROJECTION_SCHEMA_VERSION: u32 = 1;
const UPSTREAM_REPOSITORY_KEY: &str = "github.com/akiojin/gwt";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerProjectionStore {
    schema_version: u32,
    contract_revision: u32,
    pub(super) owners: Vec<OwnerProjectionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerProjectionRecord {
    pub(super) owner: OwnerProjectionOwner,
    pub(super) fingerprint: String,
    pub(super) aggregate_count: u64,
    pub(super) last_seen: String,
    pub(super) occurrences: Vec<StoredOwnerProjectionOccurrence>,
    pub(super) source_references: Vec<OwnerProjectionSourceReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredOwnerProjectionOccurrence {
    pub(super) opaque_key: String,
    pub(super) public_marker_digest: String,
    pub(super) last_seen: String,
    pub(super) comment_audit: StoredCommentAudit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredCommentAudit {
    pub(super) completeness: StoredCommentAuditCompleteness,
    pub(super) physical_comments: Vec<StoredCommentRef>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum StoredCommentAuditCompleteness {
    Complete,
    LegacyUnknown,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredCommentRef {
    pub(super) owner_number: u64,
    pub(super) comment_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnerProjectionSourceReference {
    pub(super) digest: String,
    pub(super) resolution_status: OwnerProjectionResolutionStatus,
    pub(super) last_seen: String,
    pub(super) occurrence_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum OwnerProjectionResolutionStatus {
    Linked,
    Created,
}

#[derive(Debug, Clone)]
pub(super) struct OwnerProjectionCommit {
    pub(super) owner: OwnerProjectionOwner,
    pub(super) fingerprint: String,
    pub(super) occurrence: StoredOwnerProjectionOccurrence,
    pub(super) source_reference_digest: String,
    pub(super) resolution_status: OwnerProjectionResolutionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OwnerProjectionCommitOutcome {
    pub(super) canonical_owner_number: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyOwnerProjectionStoreV1 {
    schema_version: u32,
    contract_revision: u32,
    owners: Vec<LegacyOwnerProjectionRecordV1>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyOwnerProjectionRecordV1 {
    owner: OwnerProjectionOwner,
    fingerprint: String,
    aggregate_count: u64,
    last_seen: String,
    occurrences: Vec<OwnerProjectionOccurrence>,
    source_references: Vec<OwnerProjectionSourceReference>,
}

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
    #[serde(default)]
    pub(super) remote_mutation_seen: bool,
    #[serde(default)]
    pub(super) intent: ResolutionAttemptIntent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum AttemptRemotePhase {
    NotSubmitted,
    Submitted,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum ResolutionAttemptIntent {
    #[default]
    Unassigned,
    CreateIssue {
        fingerprint: String,
        public_payload_digest: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_owner_number: Option<u64>,
    },
    CreateRegressionIssue {
        fingerprint: String,
        historical_owner_number: u64,
        recurrence_occurrence_keys: Vec<String>,
        recurrence_proof_digest: String,
        public_payload_digest: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created_owner_number: Option<u64>,
    },
    OccurrenceComments {
        owner_number: u64,
        occurrence_keys: Vec<String>,
        public_payload_digest: String,
    },
    ReconciliationComment {
        canonical_owner_number: u64,
        duplicate_owner_number: u64,
        public_payload_digest: String,
    },
    CloseDuplicate {
        canonical_owner_number: u64,
        duplicate_owner_number: u64,
    },
}

fn is_create_intent(intent: &ResolutionAttemptIntent) -> bool {
    matches!(
        intent,
        ResolutionAttemptIntent::CreateIssue { .. }
            | ResolutionAttemptIntent::CreateRegressionIssue { .. }
    )
}

fn created_owner_number(intent: &ResolutionAttemptIntent) -> Result<Option<u64>, SpecOpsError> {
    match intent {
        ResolutionAttemptIntent::CreateIssue {
            created_owner_number,
            ..
        }
        | ResolutionAttemptIntent::CreateRegressionIssue {
            created_owner_number,
            ..
        } => Ok(*created_owner_number),
        _ => Err(invalid("created owner number requires a create intent")),
    }
}

fn set_created_owner_number(
    intent: &mut ResolutionAttemptIntent,
    owner_number: u64,
) -> Result<(), SpecOpsError> {
    let created_owner_number = match intent {
        ResolutionAttemptIntent::CreateIssue {
            created_owner_number,
            ..
        }
        | ResolutionAttemptIntent::CreateRegressionIssue {
            created_owner_number,
            ..
        } => created_owner_number,
        _ => return Err(invalid("created owner number requires a create intent")),
    };
    match created_owner_number {
        Some(existing) if *existing != owner_number => {
            Err(invalid("created owner number cannot change after readback"))
        }
        Some(_) => Ok(()),
        None => {
            *created_owner_number = Some(owner_number);
            Ok(())
        }
    }
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

pub(super) fn owner_projection_path() -> PathBuf {
    owner_projection_dir().join("projection.json")
}

fn owner_projection_dir() -> PathBuf {
    let repository_key = digest_fields(
        "gwt.improvement.owner-projection.repository-key.v1",
        &[UPSTREAM_REPOSITORY_KEY],
    );
    gwt_core::paths::gwt_home()
        .join("improvements")
        .join("owner-projections")
        .join(repository_key)
}

pub(super) fn read_owner_projection_contract() -> Result<OwnerProjectionSnapshot, SpecOpsError> {
    let store = load_owner_projection()?;
    Ok(OwnerProjectionSnapshot {
        contract_revision: store.contract_revision,
        owners: store
            .owners
            .into_iter()
            .map(|record| OwnerProjectionAggregate {
                owner: record.owner,
                fingerprint: record.fingerprint,
                aggregate_count: record.aggregate_count,
                last_seen: record.last_seen,
                occurrences: record
                    .occurrences
                    .into_iter()
                    .map(|occurrence| OwnerProjectionOccurrence {
                        opaque_key: occurrence.opaque_key,
                        public_marker_digest: occurrence.public_marker_digest,
                        last_seen: occurrence.last_seen,
                    })
                    .collect(),
            })
            .collect(),
    })
}

pub(super) fn load_owner_projection() -> Result<OwnerProjectionStore, SpecOpsError> {
    with_owner_projection_lock(load_owner_projection_unlocked)
}

#[cfg(test)]
pub(super) fn commit_owner_projection(
    commit: OwnerProjectionCommit,
) -> Result<OwnerProjectionCommitOutcome, SpecOpsError> {
    commit_owner_projection_batch_inner(vec![commit], None, false).map(|mut outcomes| {
        outcomes
            .pop()
            .expect("one projection commit produces one outcome")
    })
}

pub(super) fn commit_owner_projection_batch(
    commits: Vec<OwnerProjectionCommit>,
    expected_canonical_owner_number: u64,
) -> Result<Vec<OwnerProjectionCommitOutcome>, SpecOpsError> {
    commit_owner_projection_batch_inner(commits, Some(expected_canonical_owner_number), false)
}

#[cfg(test)]
pub(super) fn fail_next_owner_projection_commit() -> Result<(), SpecOpsError> {
    with_owner_projection_lock(|| {
        fs::write(owner_projection_dir().join(".fail-next-commit"), b"failure")
            .map_err(io_as_spec_error)
    })
}

#[cfg(test)]
pub(super) fn commit_owner_projection_before_persist(
    commit: OwnerProjectionCommit,
) -> Result<OwnerProjectionCommitOutcome, SpecOpsError> {
    commit_owner_projection_batch_inner(vec![commit], None, true).map(|mut outcomes| {
        outcomes
            .pop()
            .expect("one projection commit produces one outcome")
    })
}

fn commit_owner_projection_batch_inner(
    commits: Vec<OwnerProjectionCommit>,
    expected_canonical_owner_number: Option<u64>,
    fail_before_persist: bool,
) -> Result<Vec<OwnerProjectionCommitOutcome>, SpecOpsError> {
    if commits.is_empty() {
        return Err(invalid("owner projection commit batch must not be empty"));
    }
    with_owner_projection_lock(|| {
        let mut store = load_owner_projection_unlocked()?;
        let outcomes = commits
            .into_iter()
            .map(|commit| apply_owner_projection_commit(&mut store, commit))
            .collect::<Result<Vec<_>, _>>()?;
        if expected_canonical_owner_number.is_some_and(|expected| {
            outcomes
                .iter()
                .any(|outcome| outcome.canonical_owner_number != expected)
        }) {
            return Err(invalid(
                "owner lost canonical election during projection commit",
            ));
        }
        validate_owner_projection(&store)?;
        #[cfg(test)]
        {
            let failure_marker = owner_projection_dir().join(".fail-next-commit");
            if failure_marker.exists() {
                fs::remove_file(failure_marker).map_err(io_as_spec_error)?;
                return Err(invalid("injected owner projection commit failure"));
            }
        }
        if fail_before_persist {
            return Err(invalid("injected failure before owner projection commit"));
        }
        save_owner_projection_unlocked(&store)?;
        Ok(outcomes)
    })
}

fn load_owner_projection_unlocked() -> Result<OwnerProjectionStore, SpecOpsError> {
    let path = owner_projection_path();
    if !path.exists() {
        return Ok(OwnerProjectionStore {
            schema_version: OWNER_PROJECTION_SCHEMA_VERSION,
            contract_revision: OWNER_PROJECTION_CONTRACT_REVISION,
            owners: Vec::new(),
        });
    }
    let bytes = fs::read(&path).map_err(io_as_spec_error)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(serde_as_spec_error)?;
    let schema_version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| invalid("owner projection schema version is missing or invalid"))?;
    let store = match schema_version {
        OWNER_PROJECTION_SCHEMA_VERSION => {
            serde_json::from_value(value).map_err(serde_as_spec_error)?
        }
        LEGACY_OWNER_PROJECTION_SCHEMA_VERSION => {
            let legacy: LegacyOwnerProjectionStoreV1 =
                serde_json::from_value(value).map_err(serde_as_spec_error)?;
            migrate_owner_projection_v1(legacy)?
        }
        unsupported => {
            return Err(invalid(&format!(
                "unsupported owner projection schema version: {unsupported}"
            )))
        }
    };
    validate_owner_projection(&store)?;
    Ok(store)
}

fn migrate_owner_projection_v1(
    legacy: LegacyOwnerProjectionStoreV1,
) -> Result<OwnerProjectionStore, SpecOpsError> {
    if legacy.schema_version != LEGACY_OWNER_PROJECTION_SCHEMA_VERSION {
        return Err(invalid("legacy owner projection schema version is invalid"));
    }
    if legacy.contract_revision != OWNER_PROJECTION_CONTRACT_REVISION {
        return Err(invalid(
            "legacy owner projection contract revision is invalid",
        ));
    }
    Ok(OwnerProjectionStore {
        schema_version: OWNER_PROJECTION_SCHEMA_VERSION,
        contract_revision: legacy.contract_revision,
        owners: legacy
            .owners
            .into_iter()
            .map(|record| OwnerProjectionRecord {
                owner: record.owner,
                fingerprint: record.fingerprint,
                aggregate_count: record.aggregate_count,
                last_seen: record.last_seen,
                occurrences: record
                    .occurrences
                    .into_iter()
                    .map(|occurrence| StoredOwnerProjectionOccurrence {
                        opaque_key: occurrence.opaque_key,
                        public_marker_digest: occurrence.public_marker_digest,
                        last_seen: occurrence.last_seen,
                        comment_audit: StoredCommentAudit {
                            completeness: StoredCommentAuditCompleteness::LegacyUnknown,
                            physical_comments: Vec::new(),
                        },
                    })
                    .collect(),
                source_references: record.source_references,
            })
            .collect(),
    })
}

fn save_owner_projection_unlocked(store: &OwnerProjectionStore) -> Result<(), SpecOpsError> {
    validate_owner_projection(store)?;
    let path = owner_projection_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_as_spec_error)?;
    }
    let bytes = serde_json::to_vec_pretty(store).map_err(serde_as_spec_error)?;
    write_atomic_and_sync_parent(&path, &bytes).map_err(io_as_spec_error)
}

fn with_owner_projection_lock<T>(
    operation: impl FnOnce() -> Result<T, SpecOpsError>,
) -> Result<T, SpecOpsError> {
    let directory = owner_projection_dir();
    fs::create_dir_all(&directory).map_err(io_as_spec_error)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(directory.join(".lock"))
        .map_err(io_as_spec_error)?;
    gwt_core::operation_deadline::lock_exclusive(&lock).map_err(io_as_spec_error)?;
    let result = operation();
    let unlock_result = FileExt::unlock(&lock).map_err(io_as_spec_error);
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn apply_owner_projection_commit(
    store: &mut OwnerProjectionStore,
    mut commit: OwnerProjectionCommit,
) -> Result<OwnerProjectionCommitOutcome, SpecOpsError> {
    validate_owner_projection_owner(&commit.owner)?;
    validate_fingerprint(&commit.fingerprint)?;
    validate_occurrence(&commit.fingerprint, &commit.occurrence)?;
    validate_commit_comment_audit(
        &commit.owner,
        commit.resolution_status,
        &commit.occurrence.comment_audit,
    )?;
    validate_hex_digest(
        "owner projection source reference",
        &commit.source_reference_digest,
    )?;

    let existing_occurrence_index = store.owners.iter().position(|record| {
        record
            .occurrences
            .iter()
            .any(|occurrence| occurrence.opaque_key == commit.occurrence.opaque_key)
    });
    if let Some(index) = existing_occurrence_index {
        let record = &store.owners[index];
        let existing = record
            .occurrences
            .iter()
            .find(|occurrence| occurrence.opaque_key == commit.occurrence.opaque_key)
            .expect("occurrence index was located");
        let covering_sources = record
            .source_references
            .iter()
            .filter(|source| {
                source
                    .occurrence_keys
                    .iter()
                    .any(|key| key == &existing.opaque_key)
            })
            .map(|source| source.digest.as_str())
            .collect::<Vec<_>>();
        let same_binding = record.fingerprint == commit.fingerprint
            && existing.public_marker_digest == commit.occurrence.public_marker_digest
            && covering_sources == [commit.source_reference_digest.as_str()];
        if !same_binding {
            return Err(invalid("conflicting owner projection occurrence"));
        }
        if record.owner.number != commit.owner.number {
            if commit.owner.number > record.owner.number {
                let canonical_owner_number = record.owner.number;
                let record = &mut store.owners[index];
                let existing = record
                    .occurrences
                    .iter_mut()
                    .find(|occurrence| occurrence.opaque_key == commit.occurrence.opaque_key)
                    .expect("occurrence index was located");
                existing.last_seen =
                    latest_timestamp(&existing.last_seen, &commit.occurrence.last_seen)?;
                existing.comment_audit =
                    merge_comment_audits(&existing.comment_audit, &commit.occurrence.comment_audit);
                normalize_owner_projection(store)?;
                return Ok(OwnerProjectionCommitOutcome {
                    canonical_owner_number,
                });
            }
            commit.occurrence.comment_audit =
                merge_comment_audits(&existing.comment_audit, &commit.occurrence.comment_audit);
            let record = &mut store.owners[index];
            record
                .occurrences
                .retain(|occurrence| occurrence.opaque_key != commit.occurrence.opaque_key);
            for source in &mut record.source_references {
                source
                    .occurrence_keys
                    .retain(|key| key != &commit.occurrence.opaque_key);
            }
            record
                .source_references
                .retain(|source| !source.occurrence_keys.is_empty());
            if record.occurrences.is_empty() {
                store.owners.remove(index);
            }
        }
    }

    let record = if let Some(index) = store.owners.iter().position(|record| {
        record.owner.number == commit.owner.number && record.fingerprint == commit.fingerprint
    }) {
        &mut store.owners[index]
    } else {
        store.owners.push(OwnerProjectionRecord {
            owner: commit.owner.clone(),
            fingerprint: commit.fingerprint.clone(),
            aggregate_count: 0,
            last_seen: commit.occurrence.last_seen.clone(),
            occurrences: Vec::new(),
            source_references: Vec::new(),
        });
        store.owners.last_mut().expect("record was inserted")
    };
    let stored_verified_at =
        chrono::DateTime::parse_from_rfc3339(&record.owner.readback_verified_at)
            .map_err(|_| invalid("owner projection readback_verified_at must be RFC3339"))?;
    let incoming_verified_at =
        chrono::DateTime::parse_from_rfc3339(&commit.owner.readback_verified_at)
            .map_err(|_| invalid("owner projection readback_verified_at must be RFC3339"))?;
    if incoming_verified_at > stored_verified_at {
        record.owner = commit.owner.clone();
    } else if incoming_verified_at == stored_verified_at
        && (record.owner.kind != commit.owner.kind
            || record.owner.active != commit.owner.active
            || record.owner.title != commit.owner.title)
    {
        return Err(invalid("conflicting owner projection readback state"));
    }

    if let Some(existing) = record
        .occurrences
        .iter_mut()
        .find(|occurrence| occurrence.opaque_key == commit.occurrence.opaque_key)
    {
        existing.last_seen = latest_timestamp(&existing.last_seen, &commit.occurrence.last_seen)?;
        existing.comment_audit =
            merge_comment_audits(&existing.comment_audit, &commit.occurrence.comment_audit);
    } else {
        record.occurrences.push(commit.occurrence.clone());
    }
    if let Some(source) = record
        .source_references
        .iter_mut()
        .find(|source| source.digest == commit.source_reference_digest)
    {
        source.resolution_status =
            merge_resolution_status(source.resolution_status, commit.resolution_status)?;
        source.last_seen = latest_timestamp(&source.last_seen, &commit.occurrence.last_seen)?;
        if !source
            .occurrence_keys
            .iter()
            .any(|key| key == &commit.occurrence.opaque_key)
        {
            source
                .occurrence_keys
                .push(commit.occurrence.opaque_key.clone());
        }
    } else {
        record
            .source_references
            .push(OwnerProjectionSourceReference {
                digest: commit.source_reference_digest,
                resolution_status: commit.resolution_status,
                last_seen: commit.occurrence.last_seen.clone(),
                occurrence_keys: vec![commit.occurrence.opaque_key],
            });
    }
    record.last_seen = latest_timestamp(&record.last_seen, &commit.occurrence.last_seen)?;
    normalize_owner_projection(store)?;
    Ok(OwnerProjectionCommitOutcome {
        canonical_owner_number: commit.owner.number,
    })
}

fn merge_resolution_status(
    current: OwnerProjectionResolutionStatus,
    incoming: OwnerProjectionResolutionStatus,
) -> Result<OwnerProjectionResolutionStatus, SpecOpsError> {
    match (current, incoming) {
        (OwnerProjectionResolutionStatus::Linked, OwnerProjectionResolutionStatus::Linked) => {
            Ok(OwnerProjectionResolutionStatus::Linked)
        }
        (OwnerProjectionResolutionStatus::Created, OwnerProjectionResolutionStatus::Created) => {
            Ok(OwnerProjectionResolutionStatus::Created)
        }
        (OwnerProjectionResolutionStatus::Created, OwnerProjectionResolutionStatus::Linked) => {
            Ok(OwnerProjectionResolutionStatus::Created)
        }
        (OwnerProjectionResolutionStatus::Linked, OwnerProjectionResolutionStatus::Created) => {
            Err(invalid("conflicting owner projection resolution status"))
        }
    }
}

fn normalize_owner_projection(store: &mut OwnerProjectionStore) -> Result<(), SpecOpsError> {
    for record in &mut store.owners {
        record
            .occurrences
            .sort_by(|left, right| left.opaque_key.cmp(&right.opaque_key));
        for occurrence in &mut record.occurrences {
            occurrence.comment_audit.physical_comments.sort_unstable();
            occurrence.comment_audit.physical_comments.dedup();
        }
        record.source_references.sort_by(|left, right| {
            left.digest
                .cmp(&right.digest)
                .then_with(|| left.last_seen.cmp(&right.last_seen))
        });
        for source in &mut record.source_references {
            source.occurrence_keys.sort();
            source.occurrence_keys.dedup();
            let mut latest: Option<String> = None;
            for key in &source.occurrence_keys {
                let occurrence = record
                    .occurrences
                    .iter()
                    .find(|occurrence| &occurrence.opaque_key == key)
                    .ok_or_else(|| {
                        invalid("owner projection source references an unknown occurrence")
                    })?;
                latest = Some(match latest {
                    Some(current) => latest_timestamp(&current, &occurrence.last_seen)?,
                    None => occurrence.last_seen.clone(),
                });
            }
            source.last_seen = latest.ok_or_else(|| {
                invalid("owner projection source reference requires occurrence coverage")
            })?;
        }
        record.aggregate_count = record.occurrences.len() as u64;
        record.last_seen = record
            .occurrences
            .iter()
            .try_fold(None::<String>, |latest, occurrence| {
                Ok::<_, SpecOpsError>(Some(match latest {
                    Some(current) => latest_timestamp(&current, &occurrence.last_seen)?,
                    None => occurrence.last_seen.clone(),
                }))
            })?
            .ok_or_else(|| invalid("owner projection aggregate requires an occurrence"))?;
    }
    store.owners.sort_by(owner_projection_record_order);
    Ok(())
}

fn validate_owner_projection(store: &OwnerProjectionStore) -> Result<(), SpecOpsError> {
    if store.schema_version != OWNER_PROJECTION_SCHEMA_VERSION {
        return Err(invalid(&format!(
            "unsupported owner projection schema version: {}",
            store.schema_version
        )));
    }
    if store.contract_revision != OWNER_PROJECTION_CONTRACT_REVISION {
        return Err(invalid(&format!(
            "unsupported owner projection contract revision: {}",
            store.contract_revision
        )));
    }

    let mut owner_keys = BTreeSet::new();
    let mut global_occurrences = BTreeMap::new();
    if store
        .owners
        .windows(2)
        .any(|pair| owner_projection_record_order(&pair[0], &pair[1]).is_ge())
    {
        return Err(invalid(
            "owner projection owners are not in canonical order",
        ));
    }
    for record in &store.owners {
        validate_owner_projection_owner(&record.owner)?;
        validate_fingerprint(&record.fingerprint)?;
        validate_timestamp("owner projection last_seen", &record.last_seen)?;
        if record.aggregate_count != record.occurrences.len() as u64 {
            return Err(invalid(
                "owner projection aggregate_count does not match unique occurrences",
            ));
        }
        if record.occurrences.is_empty() {
            return Err(invalid("owner projection aggregate requires an occurrence"));
        }
        if record
            .occurrences
            .windows(2)
            .any(|pair| pair[0].opaque_key >= pair[1].opaque_key)
        {
            return Err(invalid(
                "owner projection occurrences are not in canonical order",
            ));
        }
        if !owner_keys.insert((record.owner.number, record.fingerprint.as_str())) {
            return Err(invalid("duplicate owner projection aggregate"));
        }

        let occurrence_keys = record
            .occurrences
            .iter()
            .map(|occurrence| occurrence.opaque_key.as_str())
            .collect::<BTreeSet<_>>();
        if occurrence_keys.len() != record.occurrences.len() {
            return Err(invalid("duplicate owner projection occurrence"));
        }
        let mut occurrence_coverage = BTreeMap::<&str, usize>::new();
        for occurrence in &record.occurrences {
            validate_occurrence(&record.fingerprint, occurrence)?;
            if global_occurrences
                .insert(occurrence.opaque_key.as_str(), record.owner.number)
                .is_some()
            {
                return Err(invalid("conflicting owner projection occurrence"));
            }
            occurrence_coverage.insert(occurrence.opaque_key.as_str(), 0);
        }

        let mut source_digests = BTreeSet::new();
        if record
            .source_references
            .windows(2)
            .any(|pair| pair[0].digest >= pair[1].digest)
        {
            return Err(invalid(
                "owner projection source references are not in canonical order",
            ));
        }
        for source in &record.source_references {
            validate_hex_digest("owner projection source reference", &source.digest)?;
            validate_timestamp("owner projection source last_seen", &source.last_seen)?;
            if !source_digests.insert(source.digest.as_str()) {
                return Err(invalid("duplicate owner projection source reference"));
            }
            if source.occurrence_keys.is_empty() {
                return Err(invalid(
                    "owner projection source reference requires occurrence coverage",
                ));
            }
            if source
                .occurrence_keys
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            {
                return Err(invalid(
                    "owner projection source occurrence coverage is not in canonical order",
                ));
            }
            let mut covered = BTreeSet::new();
            let mut source_latest: Option<String> = None;
            for key in &source.occurrence_keys {
                if !covered.insert(key.as_str()) {
                    return Err(invalid(
                        "duplicate owner projection source occurrence coverage",
                    ));
                }
                let Some(count) = occurrence_coverage.get_mut(key.as_str()) else {
                    return Err(invalid(
                        "owner projection source references an unknown occurrence",
                    ));
                };
                *count += 1;
                let occurrence = record
                    .occurrences
                    .iter()
                    .find(|occurrence| occurrence.opaque_key == *key)
                    .expect("covered occurrence was validated above");
                validate_comment_audit_for_resolution(
                    &record.owner,
                    source.resolution_status,
                    &occurrence.comment_audit,
                )?;
                source_latest = Some(match source_latest {
                    Some(current) => latest_timestamp(&current, &occurrence.last_seen)?,
                    None => occurrence.last_seen.clone(),
                });
            }
            if !timestamps_equal(
                &source.last_seen,
                source_latest
                    .as_deref()
                    .expect("source occurrence coverage is non-empty"),
            )? {
                return Err(invalid(
                    "owner projection source last_seen does not match its occurrences",
                ));
            }
        }
        if occurrence_coverage.values().any(|count| *count != 1) {
            return Err(invalid(
                "owner projection occurrence must bind exactly one source reference",
            ));
        }
        let record_latest = record
            .occurrences
            .iter()
            .try_fold(None::<String>, |latest, occurrence| {
                Ok::<_, SpecOpsError>(Some(match latest {
                    Some(current) => latest_timestamp(&current, &occurrence.last_seen)?,
                    None => occurrence.last_seen.clone(),
                }))
            })?
            .expect("aggregate occurrence is non-empty");
        if !timestamps_equal(&record.last_seen, &record_latest)? {
            return Err(invalid(
                "owner projection last_seen does not match its occurrences",
            ));
        }
    }
    Ok(())
}

fn owner_projection_record_order(
    left: &OwnerProjectionRecord,
    right: &OwnerProjectionRecord,
) -> std::cmp::Ordering {
    left.owner
        .number
        .cmp(&right.owner.number)
        .then_with(|| left.owner.kind.cmp(&right.owner.kind))
        .then_with(|| left.fingerprint.cmp(&right.fingerprint))
}

fn validate_owner_projection_owner(owner: &OwnerProjectionOwner) -> Result<(), SpecOpsError> {
    if owner.number == 0 {
        return Err(invalid("owner projection number must be greater than zero"));
    }
    if owner.title.trim().is_empty() {
        return Err(invalid("owner projection title must not be empty"));
    }
    let expected_url = format!("https://github.com/akiojin/gwt/issues/{}", owner.number);
    if owner.url != expected_url {
        return Err(invalid("owner projection URL is not canonical"));
    }
    validate_timestamp(
        "owner projection readback_verified_at",
        &owner.readback_verified_at,
    )
}

fn validate_occurrence(
    fingerprint: &str,
    occurrence: &StoredOwnerProjectionOccurrence,
) -> Result<(), SpecOpsError> {
    let Some(digest) = occurrence.opaque_key.strip_prefix("occ:v1:") else {
        return Err(invalid("invalid owner projection occurrence key"));
    };
    validate_hex_digest("owner projection occurrence key", digest)?;
    validate_hex_digest(
        "owner projection public marker digest",
        &occurrence.public_marker_digest,
    )?;
    if occurrence.public_marker_digest
        != owner_projection_public_marker_digest(fingerprint, &occurrence.opaque_key)
    {
        return Err(invalid(
            "owner projection public marker digest does not match its occurrence",
        ));
    }
    validate_timestamp(
        "owner projection occurrence last_seen",
        &occurrence.last_seen,
    )?;
    validate_stored_comment_audit(&occurrence.comment_audit)
}

fn validate_stored_comment_audit(audit: &StoredCommentAudit) -> Result<(), SpecOpsError> {
    if audit
        .physical_comments
        .iter()
        .any(|reference| reference.owner_number == 0 || reference.comment_id == 0)
        || audit
            .physical_comments
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid(
            "owner projection comment audit must contain sorted unique positive references",
        ));
    }
    if audit.completeness == StoredCommentAuditCompleteness::Complete
        && audit.physical_comments.is_empty()
    {
        return Err(invalid(
            "complete owner projection comment audit requires a physical comment",
        ));
    }
    if audit.completeness == StoredCommentAuditCompleteness::NotApplicable
        && !audit.physical_comments.is_empty()
    {
        return Err(invalid(
            "not-applicable owner projection comment audit must be empty",
        ));
    }
    Ok(())
}

fn validate_commit_comment_audit(
    owner: &OwnerProjectionOwner,
    resolution_status: OwnerProjectionResolutionStatus,
    audit: &StoredCommentAudit,
) -> Result<(), SpecOpsError> {
    match audit.completeness {
        StoredCommentAuditCompleteness::Complete => {
            if resolution_status != OwnerProjectionResolutionStatus::Linked {
                return Err(invalid(
                    "complete owner projection comment audit requires a linked binding",
                ));
            }
            if audit
                .physical_comments
                .iter()
                .any(|reference| reference.owner_number != owner.number)
            {
                return Err(invalid(
                    "complete owner projection comment audit must match the incoming owner",
                ));
            }
        }
        StoredCommentAuditCompleteness::LegacyUnknown => {
            return Err(invalid(
                "legacy-unknown owner projection audit is migration-only",
            ));
        }
        StoredCommentAuditCompleteness::NotApplicable => {
            if resolution_status != OwnerProjectionResolutionStatus::Created
                || !audit.physical_comments.is_empty()
            {
                return Err(invalid(
                    "not-applicable owner projection comment audit requires a created binding",
                ));
            }
        }
    }
    Ok(())
}

fn validate_comment_audit_for_resolution(
    owner: &OwnerProjectionOwner,
    resolution_status: OwnerProjectionResolutionStatus,
    audit: &StoredCommentAudit,
) -> Result<(), SpecOpsError> {
    match (resolution_status, audit.completeness) {
        (
            OwnerProjectionResolutionStatus::Linked,
            StoredCommentAuditCompleteness::NotApplicable,
        ) => Err(invalid(
            "linked owner projection requires comment audit coverage",
        )),
        (OwnerProjectionResolutionStatus::Linked, StoredCommentAuditCompleteness::Complete)
            if !audit
                .physical_comments
                .iter()
                .any(|reference| reference.owner_number == owner.number) =>
        {
            Err(invalid(
                "complete linked owner projection comment audit must cover the canonical owner",
            ))
        }
        _ => Ok(()),
    }
}

fn merge_comment_audits(
    existing: &StoredCommentAudit,
    incoming: &StoredCommentAudit,
) -> StoredCommentAudit {
    let completeness = if existing.completeness == StoredCommentAuditCompleteness::LegacyUnknown
        || incoming.completeness == StoredCommentAuditCompleteness::LegacyUnknown
    {
        StoredCommentAuditCompleteness::LegacyUnknown
    } else if existing.completeness == StoredCommentAuditCompleteness::Complete
        || incoming.completeness == StoredCommentAuditCompleteness::Complete
    {
        StoredCommentAuditCompleteness::Complete
    } else {
        StoredCommentAuditCompleteness::NotApplicable
    };
    let mut physical_comments = existing.physical_comments.clone();
    physical_comments.extend(incoming.physical_comments.iter().copied());
    physical_comments.sort_unstable();
    physical_comments.dedup();
    StoredCommentAudit {
        completeness,
        physical_comments,
    }
}

fn validate_fingerprint(fingerprint: &str) -> Result<(), SpecOpsError> {
    let Some(digest) = fingerprint.strip_prefix("v2:") else {
        return Err(invalid("invalid owner projection fingerprint"));
    };
    validate_hex_digest("owner projection fingerprint", digest)
}

fn validate_hex_digest(field: &str, value: &str) -> Result<(), SpecOpsError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(invalid(&format!("invalid {field}")))
    }
}

fn validate_timestamp(field: &str, value: &str) -> Result<(), SpecOpsError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| invalid(&format!("{field} must be RFC3339")))
}

fn latest_timestamp(left: &str, right: &str) -> Result<String, SpecOpsError> {
    let left_value = chrono::DateTime::parse_from_rfc3339(left)
        .map_err(|_| invalid("owner projection timestamp must be RFC3339"))?;
    let right_value = chrono::DateTime::parse_from_rfc3339(right)
        .map_err(|_| invalid("owner projection timestamp must be RFC3339"))?;
    Ok(if right_value > left_value {
        right.to_string()
    } else {
        left.to_string()
    })
}

fn timestamps_equal(left: &str, right: &str) -> Result<bool, SpecOpsError> {
    let left_value = chrono::DateTime::parse_from_rfc3339(left)
        .map_err(|_| invalid("owner projection timestamp must be RFC3339"))?;
    let right_value = chrono::DateTime::parse_from_rfc3339(right)
        .map_err(|_| invalid("owner projection timestamp must be RFC3339"))?;
    Ok(left_value == right_value)
}

pub(super) fn load_and_repair(repo_root: &Path) -> Result<CandidateStore, SpecOpsError> {
    with_store_lock(repo_root, || load_and_repair_unlocked(repo_root))
}

pub(super) fn source_scope_nonce(repo_root: &Path) -> Result<String, SpecOpsError> {
    load_and_repair(repo_root)?
        .source_scope_nonce
        .ok_or_else(|| invalid("candidate store source scope nonce is missing after repair"))
}

#[cfg(test)]
pub(super) fn fail_next_created_owner_number_save(repo_root: &Path) -> Result<(), SpecOpsError> {
    let marker = created_owner_number_save_failure_marker(repo_root);
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent).map_err(io_as_spec_error)?;
    }
    fs::write(marker, b"failure").map_err(io_as_spec_error)
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
            candidate.blocked_reason = None;
            candidate.failure_subcode = None;
            candidate.retry = Some(RetryMetadata {
                retryable: true,
                remediation: "RECONCILE_REMOTE_OUTCOME".to_string(),
                failed_at: now.to_rfc3339(),
            });
            transition_candidate(candidate, CandidateState::RemoteOutcomeUnknown)?;
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
        remote_mutation_seen: false,
        intent: ResolutionAttemptIntent::Unassigned,
    };
    candidate.attempt = Some(lease.clone());
    candidate.updated_at = now.to_rfc3339();
    Ok(AttemptLeaseDecision::Acquired(lease))
}

pub(super) fn mark_attempt_submitted(
    store: &mut CandidateStore,
    candidate_id: &str,
    attempt_id: &str,
    intent: ResolutionAttemptIntent,
) -> Result<(), SpecOpsError> {
    if intent == ResolutionAttemptIntent::Unassigned {
        return Err(invalid("promotion attempt intent must be assigned"));
    }
    let candidate = store
        .candidates
        .iter_mut()
        .find(|candidate| candidate.id == candidate_id)
        .ok_or_else(|| invalid("candidate not found"))?;
    let attempt = candidate
        .attempt
        .as_ref()
        .filter(|attempt| attempt.attempt_id == attempt_id)
        .ok_or_else(|| invalid("promotion attempt lease is stale"))?;
    if attempt.remote_phase == AttemptRemotePhase::Submitted && attempt.intent != intent {
        return Err(invalid(
            "promotion attempt intent cannot change after submission",
        ));
    }
    if is_create_intent(&intent)
        && candidate
            .pending_create_resolution
            .as_ref()
            .is_some_and(|root| root != &intent)
    {
        return Err(invalid(
            "promotion create intent conflicts with the pending create resolution",
        ));
    }
    if is_create_intent(&intent) && candidate.pending_create_resolution.is_none() {
        candidate.pending_create_resolution = Some(intent.clone());
    }
    let attempt = candidate
        .attempt
        .as_mut()
        .filter(|attempt| attempt.attempt_id == attempt_id)
        .expect("attempt was validated before mutation");
    attempt.intent = intent;
    attempt.remote_phase = AttemptRemotePhase::Submitted;
    attempt.remote_mutation_seen = true;
    Ok(())
}

pub(super) fn record_created_owner_number(
    store: &mut CandidateStore,
    candidate_id: &str,
    attempt_id: &str,
    owner_number: u64,
) -> Result<ResolutionAttemptLease, SpecOpsError> {
    if owner_number == 0 {
        return Err(invalid("created owner number must be positive"));
    }
    let candidate = store
        .candidates
        .iter_mut()
        .find(|candidate| candidate.id == candidate_id)
        .ok_or_else(|| invalid("candidate not found"))?;
    let attempt = candidate
        .attempt
        .as_ref()
        .filter(|attempt| attempt.attempt_id == attempt_id)
        .ok_or_else(|| invalid("promotion attempt lease is stale"))?;
    let current_create_intent = is_create_intent(&attempt.intent);
    if current_create_intent && attempt.remote_phase != AttemptRemotePhase::Submitted {
        return Err(invalid(
            "created owner number requires a submitted create intent",
        ));
    }
    let root = candidate
        .pending_create_resolution
        .as_ref()
        .or_else(|| current_create_intent.then_some(&attempt.intent))
        .ok_or_else(|| invalid("created owner number requires a pending create resolution"))?;
    let root_owner_number = created_owner_number(root)?;
    if root_owner_number.is_some_and(|existing| existing != owner_number) {
        return Err(invalid("created owner number cannot change after readback"));
    }
    if current_create_intent {
        if candidate
            .pending_create_resolution
            .as_ref()
            .is_some_and(|pending| pending != &attempt.intent)
        {
            return Err(invalid(
                "submitted create intent conflicts with the pending create resolution",
            ));
        }
        let current_owner_number = created_owner_number(&attempt.intent)?;
        if current_owner_number.is_some_and(|existing| existing != owner_number) {
            return Err(invalid("created owner number cannot change after readback"));
        }
    }

    if candidate.pending_create_resolution.is_none() {
        candidate.pending_create_resolution = Some(attempt.intent.clone());
    }
    set_created_owner_number(
        candidate
            .pending_create_resolution
            .as_mut()
            .expect("pending create resolution was validated before mutation"),
        owner_number,
    )?;
    let attempt = candidate
        .attempt
        .as_mut()
        .filter(|attempt| attempt.attempt_id == attempt_id)
        .expect("attempt was validated before mutation");
    if is_create_intent(&attempt.intent) {
        set_created_owner_number(&mut attempt.intent, owner_number)?;
    }
    Ok(attempt.clone())
}

pub(super) fn clear_pending_create_resolution(
    candidate: &mut ImprovementCandidate,
    expected: &ResolutionAttemptIntent,
) -> Result<(), SpecOpsError> {
    if !is_create_intent(expected) {
        return Err(invalid(
            "pending create resolution can only be cleared by a create intent",
        ));
    }
    if candidate.pending_create_resolution.as_ref() != Some(expected) {
        return Err(invalid(
            "pending create resolution does not match the expected create intent",
        ));
    }
    candidate.pending_create_resolution = None;
    Ok(())
}

pub(super) fn renew_attempt_lease(
    store: &mut CandidateStore,
    candidate_id: &str,
    attempt_id: &str,
    now: chrono::DateTime<chrono::Utc>,
    ttl: chrono::Duration,
) -> Result<ResolutionAttemptLease, SpecOpsError> {
    if ttl <= chrono::Duration::zero() {
        return Err(invalid("promotion attempt lease TTL must be positive"));
    }
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
    if attempt.expires_at <= now {
        return Err(invalid("promotion attempt lease has expired"));
    }
    attempt.expires_at = now + ttl;
    Ok(attempt.clone())
}

pub(super) fn complete_attempt_step(
    store: &mut CandidateStore,
    candidate_id: &str,
    attempt_id: &str,
    intent: &ResolutionAttemptIntent,
) -> Result<ResolutionAttemptLease, SpecOpsError> {
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
    if attempt.remote_phase != AttemptRemotePhase::Submitted || &attempt.intent != intent {
        return Err(invalid(
            "promotion attempt step does not match submitted intent",
        ));
    }
    attempt.remote_phase = AttemptRemotePhase::NotSubmitted;
    attempt.intent = ResolutionAttemptIntent::Unassigned;
    Ok(attempt.clone())
}

fn load_and_repair_unlocked(repo_root: &Path) -> Result<CandidateStore, SpecOpsError> {
    let canonical_path = candidate_store_path(repo_root);
    let canonical_exists = canonical_path.exists();
    let mut store = if canonical_exists {
        let raw = fs::read_to_string(&canonical_path).map_err(io_as_spec_error)?;
        serde_json::from_str(&raw).map_err(serde_as_spec_error)?
    } else {
        CandidateStore {
            schema_version: STORE_SCHEMA_VERSION,
            source_scope_nonce: Some(generate_source_scope_nonce()),
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

    let mut changed = !canonical_exists;
    match store.schema_version {
        0 | 1 => {
            migrate_pre_v2_store(&mut store)?;
            changed = true;
        }
        2 => {
            migrate_v2_store(&mut store)?;
            changed = true;
        }
        STORE_SCHEMA_VERSION => {}
        _ => unreachable!("future schema rejected above"),
    }
    validate_current_store(&store)?;
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

fn migrate_pre_v2_store(store: &mut CandidateStore) -> Result<(), SpecOpsError> {
    match store.source_scope_nonce.as_deref() {
        None => store.source_scope_nonce = Some(generate_source_scope_nonce()),
        Some(nonce) if valid_source_scope_nonce(nonce) => {}
        Some(_) => return Err(invalid("invalid improvement source scope nonce")),
    }

    for candidate in &mut store.candidates {
        if candidate.schema_version > 1 {
            return Err(invalid(&format!(
                "unsupported pre-v2 candidate schema version: {}",
                candidate.schema_version
            )));
        }
        let scalar_occurrences = candidate
            .occurrences
            .max(candidate.distinct_occurrences.len() as u64);
        if scalar_occurrences > 0 || candidate.legacy_occurrence_count.is_some() {
            candidate.legacy_occurrence_count = Some(
                candidate
                    .legacy_occurrence_count
                    .unwrap_or_default()
                    .saturating_add(scalar_occurrences),
            );
        }
        candidate.occurrences = 0;
        candidate.distinct_occurrences.clear();
        candidate.fingerprint = Some(legacy_fingerprint(candidate));
        candidate.typed_evidence = None;
        candidate.eligibility = ImprovementEligibility::NeedsEvidence;
        candidate.capture_status_generation = 0;
        candidate.capture_status_delivered_generation = 0;
        if matches!(
            candidate.state,
            CandidateState::Pending | CandidateState::OwnerResolving
        ) {
            candidate.state = CandidateState::NeedsEvidence;
        }
        candidate.blocked_reason = None;
        candidate.failure_subcode = None;
        candidate.retry = None;
        candidate.owner = None;
        candidate.resolver_snapshot = None;
        candidate.schema_version = STORE_SCHEMA_VERSION;
        validate_candidate_lifecycle(candidate)?;
    }
    store.schema_version = STORE_SCHEMA_VERSION;
    Ok(())
}

fn migrate_v2_store(store: &mut CandidateStore) -> Result<(), SpecOpsError> {
    match store.source_scope_nonce.as_deref() {
        None => return Err(invalid("improvement source scope nonce is missing")),
        Some(nonce) if valid_source_scope_nonce(nonce) => {}
        Some(_) => return Err(invalid("invalid improvement source scope nonce")),
    }
    for candidate in &mut store.candidates {
        if candidate.schema_version != 2 {
            return Err(invalid(&format!(
                "candidate schema version {} does not match store schema version 2",
                candidate.schema_version
            )));
        }
        if candidate.state == CandidateState::RemoteOutcomeUnknown && candidate.retry.is_none() {
            candidate.retry = Some(RetryMetadata {
                retryable: true,
                remediation: "REFRESH_OWNER_CORPUS".to_string(),
                failed_at: candidate.updated_at.clone(),
            });
        }
        candidate.schema_version = STORE_SCHEMA_VERSION;
        validate_candidate_lifecycle(candidate)?;
    }
    store.schema_version = STORE_SCHEMA_VERSION;
    Ok(())
}

fn validate_current_store(store: &CandidateStore) -> Result<(), SpecOpsError> {
    match store.source_scope_nonce.as_deref() {
        None => return Err(invalid("improvement source scope nonce is missing")),
        Some(nonce) if valid_source_scope_nonce(nonce) => {}
        Some(_) => return Err(invalid("invalid improvement source scope nonce")),
    }
    for candidate in &store.candidates {
        if candidate.schema_version != STORE_SCHEMA_VERSION {
            return Err(invalid(&format!(
                "candidate schema version {} does not match store schema version {}",
                candidate.schema_version, STORE_SCHEMA_VERSION
            )));
        }
        validate_candidate_lifecycle(candidate)?;
    }
    Ok(())
}

fn save_unlocked(repo_root: &Path, store: &CandidateStore) -> Result<(), SpecOpsError> {
    let path = candidate_store_path(repo_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_as_spec_error)?;
    }
    let mut persisted = store.clone();
    persisted.schema_version = STORE_SCHEMA_VERSION;
    for candidate in &mut persisted.candidates {
        candidate.schema_version = STORE_SCHEMA_VERSION;
        validate_candidate_lifecycle(candidate)?;
    }
    let bytes = serde_json::to_vec_pretty(&persisted).map_err(serde_as_spec_error)?;
    #[cfg(test)]
    {
        let failure_marker = created_owner_number_save_failure_marker(repo_root);
        let saves_created_owner_number = persisted.candidates.iter().any(|candidate| {
            candidate.attempt.as_ref().is_some_and(|attempt| {
                matches!(
                    attempt.intent,
                    ResolutionAttemptIntent::CreateIssue {
                        created_owner_number: Some(_),
                        ..
                    } | ResolutionAttemptIntent::CreateRegressionIssue {
                        created_owner_number: Some(_),
                        ..
                    }
                )
            })
        });
        if saves_created_owner_number && failure_marker.exists() {
            fs::remove_file(failure_marker).map_err(io_as_spec_error)?;
            return Err(invalid("injected created owner number source save failure"));
        }
    }
    write_atomic_and_sync_parent(&path, &bytes).map_err(io_as_spec_error)
}

#[cfg(test)]
fn created_owner_number_save_failure_marker(repo_root: &Path) -> PathBuf {
    candidate_store_path(repo_root)
        .parent()
        .expect("candidate store has an improvements parent")
        .join(".fail-next-created-owner-number-save")
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
    gwt_core::operation_deadline::lock_exclusive(&lock).map_err(io_as_spec_error)?;
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
            candidate.state = CandidateState::NeedsEvidence;
            candidate.blocked_reason = None;
            candidate.failure_subcode = None;
            candidate.retry = None;
            candidate.owner = None;
            candidate.resolver_snapshot = None;
            candidate.occurrences = 0;
            candidate.legacy_occurrence_count = Some(aggregate);
            candidate.fingerprint = Some(fingerprint);
            candidate.eligibility = ImprovementEligibility::NeedsEvidence;
            candidate.typed_evidence = None;
            candidate.distinct_occurrences.clear();
            candidate.capture_status_generation = 0;
            candidate.capture_status_delivered_generation = 0;
            candidate.attempt = None;
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

fn digest_fields(domain: &str, fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    for value in std::iter::once(domain).chain(fields.iter().copied()) {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    hex::encode(digest.finalize())
}

pub(super) fn owner_projection_source_reference_digest(
    source_scope_nonce: &str,
    candidate_id: &str,
    fingerprint: &str,
) -> String {
    digest_fields(
        "gwt.improvement.owner-projection.source-reference.v1",
        &[source_scope_nonce, candidate_id, fingerprint],
    )
}

pub(super) fn owner_projection_public_marker_digest(
    fingerprint: &str,
    occurrence_key: &str,
) -> String {
    let marker = format!("<!-- gwt:improvement-occurrence:v1 {occurrence_key} -->");
    digest_fields(
        "gwt.improvement.owner-projection.public-marker.v1",
        &[fingerprint, &marker],
    )
}

fn generate_source_scope_nonce() -> String {
    let mut material = Vec::with_capacity(16 * 3);
    for _ in 0..3 {
        material.extend_from_slice(Uuid::new_v4().as_bytes());
    }
    digest_bytes(&material)
}

fn valid_source_scope_nonce(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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

    fn projection_record(comment_audit: Option<serde_json::Value>) -> serde_json::Value {
        let fingerprint = format!("v2:{}", "a".repeat(64));
        let opaque_key = format!("occ:v1:{}", "b".repeat(64));
        let mut occurrence = json!({
            "opaque_key": opaque_key,
            "public_marker_digest": owner_projection_public_marker_digest(
                &fingerprint,
                &opaque_key
            ),
            "last_seen": "2026-07-15T08:00:00Z"
        });
        if let Some(comment_audit) = comment_audit {
            occurrence
                .as_object_mut()
                .expect("occurrence object")
                .insert("comment_audit".to_string(), comment_audit);
        }
        json!({
            "owner": {
                "number": 3164,
                "kind": "spec",
                "active": true,
                "title": "Owner projection storage test",
                "url": "https://github.com/akiojin/gwt/issues/3164",
                "readback_verified_at": "2026-07-15T08:00:00Z"
            },
            "fingerprint": fingerprint,
            "aggregate_count": 1,
            "last_seen": "2026-07-15T08:00:00Z",
            "occurrences": [occurrence],
            "source_references": [{
                "digest": "c".repeat(64),
                "resolution_status": "linked",
                "last_seen": "2026-07-15T08:00:00Z",
                "occurrence_keys": [opaque_key]
            }]
        })
    }

    fn write_projection_fixture(value: &serde_json::Value) {
        let path = owner_projection_path();
        fs::create_dir_all(path.parent().expect("projection parent"))
            .expect("projection directory");
        fs::write(
            path,
            serde_json::to_vec_pretty(value).expect("projection fixture JSON"),
        )
        .expect("projection fixture");
    }

    fn store_with_candidate() -> CandidateStore {
        let candidate = serde_json::from_value(json!({
            "schema_version": 2,
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
            source_scope_nonce: Some("00".repeat(32)),
            candidates: vec![candidate],
            legacy_import: LegacyImportState::default(),
        }
    }

    fn regression_create_intent() -> ResolutionAttemptIntent {
        ResolutionAttemptIntent::CreateRegressionIssue {
            fingerprint: "v2:typed-owner-fingerprint".to_string(),
            historical_owner_number: 45,
            recurrence_occurrence_keys: vec!["occ:v1:recurrence".to_string()],
            recurrence_proof_digest: "sha256:recurrence-proof".to_string(),
            public_payload_digest: "sha256:regression-payload".to_string(),
            created_owner_number: None,
        }
    }

    #[test]
    fn schema_one_projection_migrates_to_private_unknown_audit_on_next_persist() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        write_projection_fixture(&json!({
            "schema_version": 1,
            "contract_revision": 1,
            "owners": [projection_record(None)]
        }));

        let store = load_owner_projection().expect("load schema one projection");

        assert_eq!(store.schema_version, OWNER_PROJECTION_SCHEMA_VERSION);
        assert_eq!(store.contract_revision, 1);
        assert_eq!(
            store.owners[0].occurrences[0].comment_audit.completeness,
            StoredCommentAuditCompleteness::LegacyUnknown
        );
        assert!(store.owners[0].occurrences[0]
            .comment_audit
            .physical_comments
            .is_empty());

        save_owner_projection_unlocked(&store).expect("persist schema two projection");
        let persisted: serde_json::Value = serde_json::from_slice(
            &fs::read(owner_projection_path()).expect("persisted projection"),
        )
        .expect("persisted projection JSON");
        assert_eq!(persisted["schema_version"], 2);
        assert_eq!(
            persisted["owners"][0]["occurrences"][0]["comment_audit"],
            json!({
                "completeness": "legacy-unknown",
                "physical_comments": []
            })
        );
        let public = read_owner_projection_contract().expect("public projection");
        let public_json = serde_json::to_value(public).expect("public projection JSON");
        assert_eq!(public_json["contract_revision"], 1);
        assert!(!public_json.to_string().contains("comment_audit"));
        assert!(!public_json.to_string().contains("comment_id"));
    }

    #[test]
    fn migrated_unknown_audit_is_not_upgraded_by_one_current_owner_readback() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        write_projection_fixture(&json!({
            "schema_version": 1,
            "contract_revision": 1,
            "owners": [projection_record(None)]
        }));
        let mut store = load_owner_projection().expect("load schema one projection");
        let record = store.owners[0].clone();
        let occurrence = record.occurrences[0].clone();

        apply_owner_projection_commit(
            &mut store,
            OwnerProjectionCommit {
                owner: record.owner.clone(),
                fingerprint: record.fingerprint.clone(),
                occurrence: StoredOwnerProjectionOccurrence {
                    opaque_key: occurrence.opaque_key,
                    public_marker_digest: occurrence.public_marker_digest,
                    last_seen: occurrence.last_seen,
                    comment_audit: StoredCommentAudit {
                        completeness: StoredCommentAuditCompleteness::Complete,
                        physical_comments: vec![StoredCommentRef {
                            owner_number: record.owner.number,
                            comment_id: 10,
                        }],
                    },
                },
                source_reference_digest: record.source_references[0].digest.clone(),
                resolution_status: OwnerProjectionResolutionStatus::Linked,
            },
        )
        .expect("merge current owner readback into migrated projection");

        let audit = &store.owners[0].occurrences[0].comment_audit;
        assert_eq!(
            audit.completeness,
            StoredCommentAuditCompleteness::LegacyUnknown
        );
        assert_eq!(
            audit.physical_comments,
            vec![StoredCommentRef {
                owner_number: 3164,
                comment_id: 10
            }]
        );
    }

    #[test]
    fn schema_two_projection_requires_canonical_private_comment_audit() {
        for (name, audit) in [
            ("missing", None),
            (
                "zero-id",
                Some(json!({
                    "completeness": "complete",
                    "physical_comments": [{"owner_number": 3164, "comment_id": 0}]
                })),
            ),
            (
                "noncanonical",
                Some(json!({
                    "completeness": "complete",
                    "physical_comments": [
                        {"owner_number": 3164, "comment_id": 2},
                        {"owner_number": 3164, "comment_id": 1}
                    ]
                })),
            ),
            (
                "empty-complete",
                Some(json!({
                    "completeness": "complete",
                    "physical_comments": []
                })),
            ),
            (
                "wrong-owner",
                Some(json!({
                    "completeness": "complete",
                    "physical_comments": [{"owner_number": 9999, "comment_id": 1}]
                })),
            ),
            (
                "linked-not-applicable",
                Some(json!({
                    "completeness": "not-applicable",
                    "physical_comments": []
                })),
            ),
        ] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            write_projection_fixture(&json!({
                "schema_version": 2,
                "contract_revision": 1,
                "owners": [projection_record(audit)]
            }));

            let error = load_owner_projection().expect_err(name);

            assert!(
                error.to_string().contains("comment_audit")
                    || error.to_string().contains("comment audit"),
                "{name}: {error}"
            );
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
            AttemptLeaseDecision::Acquired(lease) => {
                let serialized = serde_json::to_value(&lease).expect("serialize lease");
                assert_eq!(serialized["intent"]["kind"], "unassigned");
                lease.attempt_id
            }
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
        let intent = ResolutionAttemptIntent::CreateIssue {
            fingerprint: "v2:typed-owner-fingerprint".to_string(),
            public_payload_digest: "sha256:typed-owner-payload".to_string(),
            created_owner_number: None,
        };
        mark_attempt_submitted(&mut store, "impr-lease", &attempt_id, intent.clone())
            .expect("mark submitted");
        assert_eq!(
            store.candidates[0]
                .attempt
                .as_ref()
                .expect("attempt")
                .intent,
            intent
        );

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
        assert_eq!(
            store.candidates[0].state,
            CandidateState::RemoteOutcomeUnknown
        );
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

    #[test]
    fn regression_create_intent_records_owner_without_changing_pinned_proof() {
        let mut store = store_with_candidate();
        let now = Utc.with_ymd_and_hms(2026, 7, 15, 0, 0, 0).unwrap();
        let attempt_id = match acquire_attempt_lease(
            &mut store,
            "impr-lease",
            "worker-a",
            now,
            Duration::seconds(30),
        )
        .expect("attempt lease")
        {
            AttemptLeaseDecision::Acquired(lease) => lease.attempt_id,
            other => panic!("expected acquired lease, got {other:?}"),
        };
        let intent = regression_create_intent();
        mark_attempt_submitted(&mut store, "impr-lease", &attempt_id, intent.clone())
            .expect("mark regression create submitted");

        let recorded = record_created_owner_number(&mut store, "impr-lease", &attempt_id, 46)
            .expect("record created regression owner");

        assert_eq!(
            recorded.intent,
            ResolutionAttemptIntent::CreateRegressionIssue {
                fingerprint: "v2:typed-owner-fingerprint".to_string(),
                historical_owner_number: 45,
                recurrence_occurrence_keys: vec!["occ:v1:recurrence".to_string()],
                recurrence_proof_digest: "sha256:recurrence-proof".to_string(),
                public_payload_digest: "sha256:regression-payload".to_string(),
                created_owner_number: Some(46),
            }
        );
        assert_eq!(
            store.candidates[0].pending_create_resolution,
            Some(recorded.intent.clone()),
            "the durable create root must receive the same monotonic owner number"
        );
        record_created_owner_number(&mut store, "impr-lease", &attempt_id, 46)
            .expect("same readback is idempotent");
        assert!(record_created_owner_number(&mut store, "impr-lease", &attempt_id, 47).is_err());
    }

    #[test]
    fn pending_create_resolution_is_serde_default_and_roundtrips() {
        let mut candidate = store_with_candidate().candidates.remove(0);
        assert!(candidate.pending_create_resolution.is_none());

        let intent = regression_create_intent();
        candidate.pending_create_resolution = Some(intent.clone());
        let serialized = serde_json::to_value(&candidate).expect("serialize candidate");
        assert_eq!(
            serialized["pending_create_resolution"]["kind"],
            "create-regression-issue"
        );

        let roundtrip: ImprovementCandidate =
            serde_json::from_value(serialized).expect("deserialize candidate");
        assert_eq!(roundtrip.pending_create_resolution, Some(intent));
    }

    #[test]
    fn create_root_survives_step_completion_and_expired_lease_takeover() {
        let mut store = store_with_candidate();
        let now = Utc.with_ymd_and_hms(2026, 7, 15, 0, 0, 0).unwrap();
        let attempt_id = match acquire_attempt_lease(
            &mut store,
            "impr-lease",
            "worker-a",
            now,
            Duration::seconds(30),
        )
        .expect("attempt lease")
        {
            AttemptLeaseDecision::Acquired(lease) => lease.attempt_id,
            other => panic!("expected acquired lease, got {other:?}"),
        };
        let intent = regression_create_intent();
        mark_attempt_submitted(&mut store, "impr-lease", &attempt_id, intent.clone())
            .expect("mark create submitted");

        complete_attempt_step(&mut store, "impr-lease", &attempt_id, &intent)
            .expect("complete create step");
        assert_eq!(
            store.candidates[0].pending_create_resolution,
            Some(intent.clone())
        );

        let takeover = acquire_attempt_lease(
            &mut store,
            "impr-lease",
            "worker-b",
            now + Duration::seconds(31),
            Duration::seconds(30),
        )
        .expect("take over completed create attempt");
        assert!(matches!(takeover, AttemptLeaseDecision::Acquired(_)));
        assert_eq!(
            store.candidates[0].pending_create_resolution,
            Some(intent),
            "lease replacement must not erase the create finalization obligation"
        );
    }

    #[test]
    fn conflicting_create_root_is_rejected_without_changing_attempt() {
        let mut store = store_with_candidate();
        let now = Utc.with_ymd_and_hms(2026, 7, 15, 0, 0, 0).unwrap();
        let attempt_id = match acquire_attempt_lease(
            &mut store,
            "impr-lease",
            "worker-a",
            now,
            Duration::seconds(30),
        )
        .expect("attempt lease")
        {
            AttemptLeaseDecision::Acquired(lease) => lease.attempt_id,
            other => panic!("expected acquired lease, got {other:?}"),
        };
        let intent = regression_create_intent();
        mark_attempt_submitted(&mut store, "impr-lease", &attempt_id, intent.clone())
            .expect("mark create submitted");
        complete_attempt_step(&mut store, "impr-lease", &attempt_id, &intent)
            .expect("complete create step");
        let before = store.candidates[0].attempt.clone();
        let conflicting = ResolutionAttemptIntent::CreateIssue {
            fingerprint: "v2:different-owner-fingerprint".to_string(),
            public_payload_digest: "sha256:different-payload".to_string(),
            created_owner_number: None,
        };

        assert!(
            mark_attempt_submitted(&mut store, "impr-lease", &attempt_id, conflicting).is_err()
        );
        assert_eq!(store.candidates[0].attempt, before);
        assert_eq!(store.candidates[0].pending_create_resolution, Some(intent));
    }

    #[test]
    fn pending_create_root_can_record_owner_after_current_step_changes() {
        let mut store = store_with_candidate();
        let now = Utc.with_ymd_and_hms(2026, 7, 15, 0, 0, 0).unwrap();
        let attempt_id = match acquire_attempt_lease(
            &mut store,
            "impr-lease",
            "worker-a",
            now,
            Duration::seconds(30),
        )
        .expect("attempt lease")
        {
            AttemptLeaseDecision::Acquired(lease) => lease.attempt_id,
            other => panic!("expected acquired lease, got {other:?}"),
        };
        let root = regression_create_intent();
        mark_attempt_submitted(&mut store, "impr-lease", &attempt_id, root.clone())
            .expect("mark create submitted");
        complete_attempt_step(&mut store, "impr-lease", &attempt_id, &root)
            .expect("complete create step");
        let reconciliation = ResolutionAttemptIntent::ReconciliationComment {
            canonical_owner_number: 46,
            duplicate_owner_number: 47,
            public_payload_digest: "sha256:reconciliation".to_string(),
        };
        mark_attempt_submitted(
            &mut store,
            "impr-lease",
            &attempt_id,
            reconciliation.clone(),
        )
        .expect("mark reconciliation submitted");

        let attempt = record_created_owner_number(&mut store, "impr-lease", &attempt_id, 47)
            .expect("record owner in durable root");

        assert_eq!(attempt.intent, reconciliation);
        assert!(matches!(
            store.candidates[0].pending_create_resolution,
            Some(ResolutionAttemptIntent::CreateRegressionIssue {
                created_owner_number: Some(47),
                ..
            })
        ));
    }

    #[test]
    fn clearing_pending_create_resolution_requires_the_exact_root() {
        let mut store = store_with_candidate();
        let root = regression_create_intent();
        store.candidates[0].pending_create_resolution = Some(root.clone());
        let different = ResolutionAttemptIntent::CreateIssue {
            fingerprint: "v2:different-owner-fingerprint".to_string(),
            public_payload_digest: "sha256:different-payload".to_string(),
            created_owner_number: None,
        };

        assert!(clear_pending_create_resolution(&mut store.candidates[0], &different).is_err());
        assert_eq!(
            store.candidates[0].pending_create_resolution,
            Some(root.clone())
        );

        clear_pending_create_resolution(&mut store.candidates[0], &root)
            .expect("clear exact durable root");
        assert!(store.candidates[0].pending_create_resolution.is_none());
    }
}
