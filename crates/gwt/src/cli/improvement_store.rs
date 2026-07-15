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
const OWNER_PROJECTION_SCHEMA_VERSION: u32 = 1;
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
    pub(super) occurrences: Vec<OwnerProjectionOccurrence>,
    pub(super) source_references: Vec<OwnerProjectionSourceReference>,
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
    pub(super) occurrence: OwnerProjectionOccurrence,
    pub(super) source_reference_digest: String,
    pub(super) resolution_status: OwnerProjectionResolutionStatus,
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
                occurrences: record.occurrences,
            })
            .collect(),
    })
}

pub(super) fn load_owner_projection() -> Result<OwnerProjectionStore, SpecOpsError> {
    with_owner_projection_lock(load_owner_projection_unlocked)
}

pub(super) fn commit_owner_projection(commit: OwnerProjectionCommit) -> Result<(), SpecOpsError> {
    commit_owner_projection_inner(commit, false)
}

#[cfg(test)]
pub(super) fn commit_owner_projection_before_persist(
    commit: OwnerProjectionCommit,
) -> Result<(), SpecOpsError> {
    commit_owner_projection_inner(commit, true)
}

fn commit_owner_projection_inner(
    commit: OwnerProjectionCommit,
    fail_before_persist: bool,
) -> Result<(), SpecOpsError> {
    with_owner_projection_lock(|| {
        let mut store = load_owner_projection_unlocked()?;
        apply_owner_projection_commit(&mut store, commit)?;
        validate_owner_projection(&store)?;
        if fail_before_persist {
            return Err(invalid("injected failure before owner projection commit"));
        }
        save_owner_projection_unlocked(&store)
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
    let store: OwnerProjectionStore =
        serde_json::from_slice(&bytes).map_err(serde_as_spec_error)?;
    validate_owner_projection(&store)?;
    Ok(store)
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
    lock.lock_exclusive().map_err(io_as_spec_error)?;
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
    commit: OwnerProjectionCommit,
) -> Result<(), SpecOpsError> {
    validate_owner_projection_owner(&commit.owner)?;
    validate_fingerprint(&commit.fingerprint)?;
    validate_occurrence(&commit.fingerprint, &commit.occurrence)?;
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
                return Err(invalid(
                    "owner projection occurrence already has a lower canonical owner",
                ));
            }
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
    Ok(())
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
    occurrence: &OwnerProjectionOccurrence,
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
    )
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
}
