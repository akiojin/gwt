#![cfg_attr(not(test), allow(dead_code))]

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};

use chrono::Utc;
use gwt_github::client::{
    ApiError as GitHubApiError, CommitComparisonStatus, CreateRepositoryIssue, IssueNumber,
    IssueState, OwnerMutationError, OwnerRepositoryClient, RepositoryActorType,
    RepositoryAuthorAssociation, RepositoryComment, RepositoryIdentity, RepositoryIssue,
    RepositoryIssueKind, ResolutionDeadline,
};
use regex::Regex;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    improvement::{
        improvement_fingerprint, post_improvement_board_status, transition_candidate,
        typed_evidence_digest, BlockedReason, CandidateState, CaptureBudgetProfile,
        DurableOwnerSnapshot, FailureSubcode, ImprovementAuditEntry, ImprovementCandidate,
        LinkedIssue, OccurrenceOrigin, OccurrenceReplayProof, OwnerCandidate, OwnerKind,
        OwnerMatchBasis, ResolverSnapshot, RetryMetadata, TypedFailureEvidence,
    },
    improvement_contract::{OwnerProjectionOwner, OwnerProjectionOwnerKind},
    improvement_store::{
        AttemptLeaseDecision, AttemptRemotePhase, OwnerProjectionCommit, OwnerProjectionRecord,
        OwnerProjectionResolutionStatus, OwnerProjectionSourceReference, OwnerProjectionStore,
        ResolutionAttemptIntent,
    },
    CliEnv,
};

pub(super) const MAX_PUBLIC_BODY_BYTES: usize = 16 * 1024;
const MAX_PUBLIC_TITLE_CHARS: usize = 180;
const MAX_PUBLIC_SUMMARY_CHARS: usize = 150;

#[derive(Debug, Clone)]
struct ReadbackVerifiedOwnerBinding {
    candidate_id: String,
    owner: DurableOwnerSnapshot,
    occurrence_key: String,
    resolution_status: CandidateState,
    last_seen: String,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
enum ProjectionCommitFailurePoint {
    BeforePersist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ContractRouteDisposition {
    ExistingOwner(IssueNumber),
    ImplementationGap,
    Aligned,
    SpecGap,
    SpecAmbiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ContractRouteMapping {
    pub(super) contract_id: String,
    pub(super) contract_schema_revision: u64,
    pub(super) failure_code: String,
    pub(super) expected_outcome: String,
    pub(super) routing_basis_revision: u64,
    pub(super) disposition: ContractRouteDisposition,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ContractRoutingRegistry {
    mappings: Vec<ContractRouteMapping>,
}

impl ContractRoutingRegistry {
    pub(super) fn new(mappings: Vec<ContractRouteMapping>) -> Self {
        Self { mappings }
    }

    fn current() -> Self {
        Self::new(vec![
            #[cfg(test)]
            ContractRouteMapping {
                contract_id: "coordination.board-status".to_string(),
                contract_schema_revision: 1,
                failure_code: "STATUS_NOT_POSTED".to_string(),
                expected_outcome: "BOARD_STATUS_POSTED".to_string(),
                routing_basis_revision: 1,
                disposition: ContractRouteDisposition::ImplementationGap,
            },
            #[cfg(test)]
            ContractRouteMapping {
                contract_id: "coordination.board-status".to_string(),
                contract_schema_revision: 1,
                failure_code: "STATUS_NOT_POSTED".to_string(),
                expected_outcome: "BOARD_STATUS_POSTED".to_string(),
                routing_basis_revision: 9,
                disposition: ContractRouteDisposition::ExistingOwner(IssueNumber(77)),
            },
        ])
    }

    fn matching_owner_numbers(&self, candidate: &ImprovementCandidate) -> Vec<IssueNumber> {
        self.matching_dispositions(candidate)
            .into_iter()
            .filter_map(|disposition| match disposition {
                ContractRouteDisposition::ExistingOwner(number) => Some(number),
                _ => None,
            })
            .collect()
    }

    fn matching_dispositions(
        &self,
        candidate: &ImprovementCandidate,
    ) -> Vec<ContractRouteDisposition> {
        let Some(evidence) = candidate.typed_evidence.as_ref() else {
            return Vec::new();
        };
        self.mappings
            .iter()
            .filter(|mapping| {
                mapping.contract_id == evidence.contract_id
                    && mapping.contract_schema_revision == evidence.contract_schema_revision
                    && mapping.failure_code == evidence.failure_code
                    && mapping.expected_outcome == evidence.expected_outcome
                    && candidate.distinct_occurrences.iter().any(|occurrence| {
                        occurrence.origin == OccurrenceOrigin::Deterministic
                            && occurrence.qualifies_unattended
                            && occurrence.producer_registry_revision.is_some()
                            && occurrence.routing_basis_revision
                                == Some(mapping.routing_basis_revision)
                    })
            })
            .map(|mapping| mapping.disposition)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
fn commit_readback_verified_binding(
    source_repo_root: &Path,
    binding: &ReadbackVerifiedOwnerBinding,
) -> Result<(), gwt_github::SpecOpsError> {
    let commit = prepare_test_owner_projection_commit(source_repo_root, binding)?;
    let outcome = super::improvement_store::commit_owner_projection(commit)?;
    if outcome.canonical_owner_number != binding.owner.number {
        return Err(owner_projection_error(
            "owner projection occurrence already has a lower canonical owner",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn commit_readback_verified_binding_for_test(
    source_repo_root: &Path,
    binding: &ReadbackVerifiedOwnerBinding,
    failure_point: ProjectionCommitFailurePoint,
) -> Result<(), gwt_github::SpecOpsError> {
    let commit = prepare_test_owner_projection_commit(source_repo_root, binding)?;
    match failure_point {
        ProjectionCommitFailurePoint::BeforePersist => {
            super::improvement_store::commit_owner_projection_before_persist(commit).map(|_| ())
        }
    }
}

#[cfg(test)]
fn prepare_test_owner_projection_commit(
    source_repo_root: &Path,
    binding: &ReadbackVerifiedOwnerBinding,
) -> Result<OwnerProjectionCommit, gwt_github::SpecOpsError> {
    let mut commit = prepare_owner_projection_commit(source_repo_root, binding)?;
    if commit.occurrence.comment_audit.completeness
        == super::improvement_store::StoredCommentAuditCompleteness::LegacyUnknown
    {
        let digest = binding
            .occurrence_key
            .strip_prefix("occ:v1:")
            .and_then(|value| value.get(..16))
            .and_then(|value| u64::from_str_radix(value, 16).ok())
            .unwrap_or(1)
            .max(1);
        commit.occurrence.comment_audit = super::improvement_store::StoredCommentAudit {
            completeness: super::improvement_store::StoredCommentAuditCompleteness::Complete,
            physical_comments: vec![super::improvement_store::StoredCommentRef {
                owner_number: binding.owner.number,
                comment_id: digest,
            }],
        };
    }
    Ok(commit)
}

#[cfg(test)]
fn prepare_owner_projection_commit(
    source_repo_root: &Path,
    binding: &ReadbackVerifiedOwnerBinding,
) -> Result<OwnerProjectionCommit, gwt_github::SpecOpsError> {
    let public_context = PublicMutationContext::for_repo(source_repo_root);
    prepare_owner_projection_commit_with_context(source_repo_root, binding, &public_context)
}

fn prepare_owner_projection_commit_with_context(
    source_repo_root: &Path,
    binding: &ReadbackVerifiedOwnerBinding,
    public_context: &PublicMutationContext,
) -> Result<OwnerProjectionCommit, gwt_github::SpecOpsError> {
    let source_store = super::improvement_store::load_and_repair(source_repo_root)?;
    let source_scope_nonce = source_store
        .source_scope_nonce
        .as_deref()
        .ok_or_else(|| owner_projection_error("source scope nonce is missing"))?;
    let candidate = source_store
        .candidates
        .iter()
        .find(|candidate| candidate.id == binding.candidate_id)
        .ok_or_else(|| owner_projection_error("candidate not found for owner projection"))?;
    let fingerprint = candidate
        .fingerprint
        .as_deref()
        .ok_or_else(|| owner_projection_error("candidate fingerprint is missing"))?;
    if binding.owner.fingerprint != fingerprint {
        return Err(owner_projection_error(
            "verified owner fingerprint does not match candidate",
        ));
    }
    if !candidate
        .distinct_occurrences
        .iter()
        .any(|occurrence| occurrence.opaque_key == binding.occurrence_key)
    {
        return Err(owner_projection_error(
            "verified owner binding occurrence is not present in candidate",
        ));
    }
    let resolution_status = match binding.resolution_status {
        CandidateState::Linked => OwnerProjectionResolutionStatus::Linked,
        CandidateState::Created => OwnerProjectionResolutionStatus::Created,
        _ => {
            return Err(owner_projection_error(
                "verified owner binding must resolve to linked or created",
            ))
        }
    };
    validate_projection_owner_for_candidate(candidate, &binding.owner, public_context)?;
    chrono::DateTime::parse_from_rfc3339(&binding.last_seen)
        .map_err(|_| owner_projection_error("owner projection last_seen must be RFC3339"))?;
    let source_reference_digest =
        super::improvement_store::owner_projection_source_reference_digest(
            source_scope_nonce,
            &candidate.id,
            fingerprint,
        );
    let public_marker_digest = super::improvement_store::owner_projection_public_marker_digest(
        fingerprint,
        &binding.occurrence_key,
    );
    Ok(OwnerProjectionCommit {
        owner: owner_projection_owner(&binding.owner),
        fingerprint: fingerprint.to_string(),
        occurrence: super::improvement_store::StoredOwnerProjectionOccurrence {
            opaque_key: binding.occurrence_key.clone(),
            public_marker_digest,
            last_seen: binding.last_seen.clone(),
            comment_audit: super::improvement_store::StoredCommentAudit {
                completeness: match resolution_status {
                    OwnerProjectionResolutionStatus::Linked => {
                        super::improvement_store::StoredCommentAuditCompleteness::LegacyUnknown
                    }
                    OwnerProjectionResolutionStatus::Created => {
                        super::improvement_store::StoredCommentAuditCompleteness::NotApplicable
                    }
                },
                physical_comments: Vec::new(),
            },
        },
        source_reference_digest,
        resolution_status,
    })
}

fn validate_projection_owner_snapshot(
    owner: &DurableOwnerSnapshot,
) -> Result<(), gwt_github::SpecOpsError> {
    if owner.number == 0 || owner.title.trim().is_empty() || !owner.active {
        return Err(owner_projection_error(
            "verified owner snapshot is not an active public owner",
        ));
    }
    let canonical_url = format!("https://github.com/akiojin/gwt/issues/{}", owner.number);
    if owner.url != canonical_url {
        return Err(owner_projection_error(
            "verified owner snapshot URL is not canonical",
        ));
    }
    chrono::DateTime::parse_from_rfc3339(&owner.readback_verified_at)
        .map(|_| ())
        .map_err(|_| owner_projection_error("verified owner readback timestamp must be RFC3339"))
}

fn validate_projection_owner_for_candidate(
    candidate: &ImprovementCandidate,
    owner: &DurableOwnerSnapshot,
    public_context: &PublicMutationContext,
) -> Result<(), gwt_github::SpecOpsError> {
    validate_projection_owner_snapshot(owner)?;
    validate_public_payload(
        &PublicIssuePayload {
            summary: "Verified upstream owner identity".to_string(),
            title: owner.title.clone(),
            body: "Verified upstream owner identity.".to_string(),
        },
        &public_context.with_candidate(candidate, &[]),
    )
    .map_err(|error| {
        owner_projection_error(&format!(
            "owner projection privacy validation failed: {error}"
        ))
    })
}

fn owner_projection_owner(owner: &DurableOwnerSnapshot) -> OwnerProjectionOwner {
    OwnerProjectionOwner {
        number: owner.number,
        kind: match owner.kind {
            OwnerKind::Issue => OwnerProjectionOwnerKind::Issue,
            OwnerKind::Spec => OwnerProjectionOwnerKind::Spec,
        },
        active: owner.active,
        title: owner.title.clone(),
        url: owner.url.clone(),
        readback_verified_at: owner.readback_verified_at.clone(),
    }
}

pub(super) fn repair_source_success_snapshots(
    source_repo_root: &Path,
) -> Result<bool, gwt_github::SpecOpsError> {
    // Projection and source stores have independent locks. Read the projection
    // fully before acquiring a source lock; cross-file atomicity is not implied.
    let projection = super::improvement_store::load_owner_projection()?;
    let source_store = super::improvement_store::load_and_repair(source_repo_root)?;
    if !source_store_needs_projection_repair(&source_store, &projection)? {
        return Ok(false);
    }
    super::improvement_store::update(source_repo_root, |store| {
        let source_scope_nonce = store
            .source_scope_nonce
            .clone()
            .ok_or_else(|| owner_projection_error("source scope nonce is missing"))?;
        let mut changed = false;
        let now = Utc::now();
        for candidate in &mut store.candidates {
            if candidate_blocks_projection_repair(candidate, now) {
                continue;
            }
            let Some(binding) =
                latest_projection_binding(&projection, &source_scope_nonce, candidate)?
            else {
                continue;
            };
            changed |= apply_projection_binding(candidate, binding)?;
        }
        Ok(changed)
    })
}

fn candidate_blocks_projection_repair(
    candidate: &ImprovementCandidate,
    now: chrono::DateTime<Utc>,
) -> bool {
    candidate.reconciliation_required
        || candidate
            .attempt
            .as_ref()
            .is_some_and(|attempt| attempt.expires_at > now)
}

fn ensure_candidate_attempt_current(
    repo_root: &Path,
    candidate: &ImprovementCandidate,
) -> Result<(), gwt_github::SpecOpsError> {
    let store = super::improvement_store::load_and_repair(repo_root)?;
    let stored = store
        .candidates
        .iter()
        .find(|stored| stored.id == candidate.id)
        .ok_or_else(|| owner_projection_error("candidate not found"))?;
    if candidate.state != CandidateState::OwnerResolving
        || candidate.dismissed_reason.is_some()
        || stored.state != candidate.state
        || stored.dismissed_reason != candidate.dismissed_reason
        || stored.audit != candidate.audit
    {
        return Err(owner_projection_error(
            "candidate changed while Owner Resolution was running",
        ));
    }
    match candidate.attempt.as_ref() {
        Some(expected) => {
            let attempt = stored
                .attempt
                .as_ref()
                .filter(|attempt| attempt.attempt_id == expected.attempt_id)
                .ok_or_else(|| owner_projection_error("owner resolution attempt lease is stale"))?;
            if attempt.expires_at <= Utc::now() {
                return Err(owner_projection_error(
                    "owner resolution attempt lease has expired",
                ));
            }
        }
        None if stored.attempt.is_some() => {
            return Err(owner_projection_error(
                "owner resolution attempt lease is stale",
            ));
        }
        None => {}
    }
    Ok(())
}

fn commit_owner_projection_and_source_success(
    repo_root: &Path,
    candidate: &ImprovementCandidate,
    projection_commits: Vec<OwnerProjectionCommit>,
    expected_owner_number: u64,
    reconciliation_cleanup_verified: bool,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    let expected_attempt_id = candidate
        .attempt
        .as_ref()
        .map(|attempt| attempt.attempt_id.clone());
    let expected_fingerprint = candidate.fingerprint.clone();
    let expected_occurrences = candidate.distinct_occurrences.clone();
    let expected_reconciliation_required = candidate.reconciliation_required;
    let expected_reconciliation_owner_numbers = candidate.reconciliation_owner_numbers.clone();
    let expected_pending_create_resolution = candidate.pending_create_resolution.clone();
    let expected_state = candidate.state;
    let expected_dismissed_reason = candidate.dismissed_reason.clone();
    let expected_audit = candidate.audit.clone();
    let candidate_id = candidate.id.clone();

    // Keep the source lock from the lease fence through both projection and
    // source persistence. A takeover cannot interleave after validation.
    super::improvement_store::update(repo_root, move |store| {
        let source_scope_nonce = store
            .source_scope_nonce
            .clone()
            .ok_or_else(|| owner_projection_error("source scope nonce is missing"))?;
        let stored = store
            .candidates
            .iter_mut()
            .find(|stored| stored.id == candidate_id)
            .ok_or_else(|| owner_projection_error("candidate not found"))?;
        if stored.fingerprint != expected_fingerprint
            || stored.distinct_occurrences != expected_occurrences
            || stored.reconciliation_required != expected_reconciliation_required
            || stored.reconciliation_owner_numbers != expected_reconciliation_owner_numbers
            || stored.pending_create_resolution != expected_pending_create_resolution
            || stored.state != expected_state
            || stored.dismissed_reason != expected_dismissed_reason
            || stored.audit != expected_audit
        {
            return Err(owner_projection_error(
                "candidate changed while Owner Resolution was running",
            ));
        }
        match expected_attempt_id.as_deref() {
            Some(expected_attempt_id) => {
                let attempt = stored
                    .attempt
                    .as_ref()
                    .filter(|attempt| attempt.attempt_id == expected_attempt_id)
                    .ok_or_else(|| {
                        owner_projection_error("owner resolution attempt lease is stale")
                    })?;
                if attempt.expires_at <= Utc::now() {
                    return Err(owner_projection_error(
                        "owner resolution attempt lease has expired",
                    ));
                }
            }
            None if stored.attempt.is_some() => {
                return Err(owner_projection_error(
                    "owner resolution attempt lease is stale",
                ));
            }
            None => {}
        }
        if stored.reconciliation_required && !reconciliation_cleanup_verified {
            return Err(owner_projection_error(
                "reconciliation cleanup is not authoritatively verified",
            ));
        }

        let outcomes = super::improvement_store::commit_owner_projection_batch(
            projection_commits,
            expected_owner_number,
        )?;
        for outcome in outcomes {
            if outcome.canonical_owner_number != expected_owner_number {
                return Err(owner_projection_error(
                    "owner lost canonical election during projection commit",
                ));
            }
        }
        let projection = super::improvement_store::load_owner_projection()?;
        let binding = latest_projection_binding(&projection, &source_scope_nonce, stored)?
            .ok_or_else(|| {
                owner_projection_error(
                    "owner projection does not cover the current source candidate",
                )
            })?;
        apply_projection_binding(stored, binding)?;
        if stored
            .owner
            .as_ref()
            .is_none_or(|owner| owner.number != expected_owner_number)
        {
            return Err(owner_projection_error(
                "source candidate resolved to an unexpected projected owner",
            ));
        }
        stored.reconciliation_required = false;
        stored.reconciliation_owner_numbers.clear();
        super::improvement::validate_candidate_lifecycle(stored)?;
        Ok(stored.clone())
    })
}

fn arm_reconciliation_required(
    repo_root: &Path,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    owner_numbers: &[u64],
) -> Result<(), gwt_github::SpecOpsError> {
    if token.candidate_id != candidate.id {
        return Err(owner_projection_error(
            "owner resolution attempt token candidate mismatch",
        ));
    }
    let expected_fingerprint = candidate.fingerprint.clone();
    let expected_occurrences = candidate.distinct_occurrences.clone();
    let mut observed_owner_numbers = owner_numbers.to_vec();
    observed_owner_numbers.sort_unstable();
    observed_owner_numbers.dedup();
    if observed_owner_numbers.len() < 2 || observed_owner_numbers[0] == 0 {
        return Err(owner_projection_error(
            "reconciliation latch requires multiple positive owner numbers",
        ));
    }
    let persisted_owner_numbers = super::improvement_store::update(repo_root, |store| {
        let stored = store
            .candidates
            .iter_mut()
            .find(|stored| stored.id == token.candidate_id)
            .ok_or_else(|| owner_projection_error("candidate not found"))?;
        if stored.fingerprint != expected_fingerprint
            || stored.distinct_occurrences != expected_occurrences
        {
            return Err(owner_projection_error(
                "candidate changed while Owner Resolution was running",
            ));
        }
        let attempt = stored
            .attempt
            .as_ref()
            .filter(|attempt| attempt.attempt_id == token.attempt_id)
            .ok_or_else(|| owner_projection_error("owner resolution attempt lease is stale"))?;
        if attempt.expires_at <= Utc::now() {
            return Err(owner_projection_error(
                "owner resolution attempt lease has expired",
            ));
        }
        stored.reconciliation_required = true;
        stored
            .reconciliation_owner_numbers
            .extend(observed_owner_numbers);
        stored.reconciliation_owner_numbers.sort_unstable();
        stored.reconciliation_owner_numbers.dedup();
        super::improvement::validate_candidate_lifecycle(stored)?;
        Ok(stored.reconciliation_owner_numbers.clone())
    })?;
    candidate.reconciliation_required = true;
    candidate.reconciliation_owner_numbers = persisted_owner_numbers;
    Ok(())
}

fn source_store_needs_projection_repair(
    store: &super::improvement::CandidateStore,
    projection: &OwnerProjectionStore,
) -> Result<bool, gwt_github::SpecOpsError> {
    let source_scope_nonce = store
        .source_scope_nonce
        .as_deref()
        .ok_or_else(|| owner_projection_error("source scope nonce is missing"))?;
    let now = Utc::now();
    for candidate in &store.candidates {
        if candidate_blocks_projection_repair(candidate, now) {
            continue;
        }
        let Some(binding) = latest_projection_binding(projection, source_scope_nonce, candidate)?
        else {
            continue;
        };
        let mut projected = candidate.clone();
        if apply_projection_binding(&mut projected, binding)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn latest_projection_binding<'a>(
    projection: &'a OwnerProjectionStore,
    source_scope_nonce: &str,
    candidate: &ImprovementCandidate,
) -> Result<
    Option<(
        &'a OwnerProjectionRecord,
        &'a OwnerProjectionSourceReference,
    )>,
    gwt_github::SpecOpsError,
> {
    if matches!(
        candidate.state,
        CandidateState::Pending
            | CandidateState::NeedsEvidence
            | CandidateState::Parked
            | CandidateState::Dismissed
    ) {
        return Ok(None);
    }
    let Some(fingerprint) = candidate.fingerprint.as_deref() else {
        return Ok(None);
    };
    let current_occurrences = candidate
        .distinct_occurrences
        .iter()
        .map(|occurrence| occurrence.opaque_key.as_str())
        .collect::<BTreeSet<_>>();
    if current_occurrences.is_empty()
        || current_occurrences.len() != candidate.distinct_occurrences.len()
    {
        return Ok(None);
    }
    let source_reference = super::improvement_store::owner_projection_source_reference_digest(
        source_scope_nonce,
        &candidate.id,
        fingerprint,
    );
    let mut matches = projection
        .owners
        .iter()
        .filter(|record| record.fingerprint == fingerprint)
        .flat_map(|record| {
            record
                .source_references
                .iter()
                .filter(|source| source.digest == source_reference)
                .map(move |source| (record, source))
        })
        .collect::<Vec<_>>();
    let projected_occurrences = matches
        .iter()
        .flat_map(|(_, source)| source.occurrence_keys.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    if !current_occurrences
        .iter()
        .all(|key| projected_occurrences.contains(key))
    {
        return Ok(None);
    }
    matches.sort_by(|(left_record, left_source), (right_record, right_source)| {
        projection_binding_order(left_record, left_source, right_record, right_source)
    });
    Ok(matches.pop())
}

fn projection_binding_order(
    left_record: &OwnerProjectionRecord,
    left_source: &OwnerProjectionSourceReference,
    right_record: &OwnerProjectionRecord,
    right_source: &OwnerProjectionSourceReference,
) -> std::cmp::Ordering {
    let left_verified =
        chrono::DateTime::parse_from_rfc3339(&left_record.owner.readback_verified_at)
            .expect("validated projection timestamp");
    let right_verified =
        chrono::DateTime::parse_from_rfc3339(&right_record.owner.readback_verified_at)
            .expect("validated projection timestamp");
    let left_seen = chrono::DateTime::parse_from_rfc3339(&left_source.last_seen)
        .expect("validated projection timestamp");
    let right_seen = chrono::DateTime::parse_from_rfc3339(&right_source.last_seen)
        .expect("validated projection timestamp");
    left_verified
        .cmp(&right_verified)
        .then_with(|| left_seen.cmp(&right_seen))
        .then_with(|| left_record.owner.number.cmp(&right_record.owner.number))
        .then_with(|| left_record.owner.kind.cmp(&right_record.owner.kind))
        .then_with(|| left_record.fingerprint.cmp(&right_record.fingerprint))
}

fn apply_projection_binding(
    candidate: &mut ImprovementCandidate,
    (record, source): (&OwnerProjectionRecord, &OwnerProjectionSourceReference),
) -> Result<bool, gwt_github::SpecOpsError> {
    if candidate.fingerprint.as_deref() != Some(record.fingerprint.as_str()) {
        return Ok(false);
    }
    let target_state = match source.resolution_status {
        OwnerProjectionResolutionStatus::Linked => CandidateState::Linked,
        OwnerProjectionResolutionStatus::Created => CandidateState::Created,
    };
    let owner = DurableOwnerSnapshot {
        number: record.owner.number,
        kind: match record.owner.kind {
            OwnerProjectionOwnerKind::Issue => OwnerKind::Issue,
            OwnerProjectionOwnerKind::Spec => OwnerKind::Spec,
        },
        title: record.owner.title.clone(),
        active: record.owner.active,
        url: record.owner.url.clone(),
        fingerprint: record.fingerprint.clone(),
        readback_verified_at: record.owner.readback_verified_at.clone(),
    };
    let linked_issue = LinkedIssue {
        number: record.owner.number,
        url: record.owner.url.clone(),
        repository: "akiojin/gwt".to_string(),
    };
    let public_binding_changed = candidate.owner.as_ref().is_none_or(|current| {
        current.number != owner.number
            || current.kind != owner.kind
            || current.title != owner.title
            || current.active != owner.active
            || current.url != owner.url
            || current.fingerprint != owner.fingerprint
    }) || candidate.linked_issue.as_ref().is_none_or(|current| {
        current.number != linked_issue.number
            || current.url != linked_issue.url
            || current.repository != linked_issue.repository
    });
    let stable_status_changed = matches!(
        candidate.state,
        CandidateState::Linked | CandidateState::Created
    ) && candidate.state != target_state;
    let core_current = candidate.state == target_state
        && candidate.owner.as_ref().is_some_and(|current| {
            current.number == owner.number
                && current.kind == owner.kind
                && current.title == owner.title
                && current.active == owner.active
                && current.url == owner.url
                && current.fingerprint == owner.fingerprint
                && current.readback_verified_at == owner.readback_verified_at
        })
        && candidate.linked_issue.as_ref().is_some_and(|current| {
            current.number == linked_issue.number
                && current.url == linked_issue.url
                && current.repository == linked_issue.repository
        });
    let already_current = core_current
        && candidate.blocked_reason.is_none()
        && candidate.failure_subcode.is_none()
        && candidate.retry.is_none()
        && candidate.attempt.is_none();
    if already_current {
        return Ok(false);
    }
    if !core_current
        && matches!(
            candidate.state,
            CandidateState::Linked | CandidateState::Created
        )
    {
        if let Some(current_owner) = candidate.owner.as_ref() {
            let current_verified = chrono::DateTime::parse_from_rfc3339(
                &current_owner.readback_verified_at,
            )
            .map_err(|_| owner_projection_error("candidate owner readback timestamp is invalid"))?;
            let projected_verified =
                chrono::DateTime::parse_from_rfc3339(&record.owner.readback_verified_at)
                    .expect("validated projection timestamp");
            let current_seen = chrono::DateTime::parse_from_rfc3339(&candidate.updated_at)
                .map_err(|_| owner_projection_error("candidate updated_at must be RFC3339"))?;
            let projected_seen = chrono::DateTime::parse_from_rfc3339(&source.last_seen)
                .expect("validated projection timestamp");
            match (projected_verified, projected_seen).cmp(&(current_verified, current_seen)) {
                std::cmp::Ordering::Less => return Ok(false),
                std::cmp::Ordering::Equal => {
                    return Err(owner_projection_error(
                        "conflicting owner binding at the same verified revision",
                    ))
                }
                std::cmp::Ordering::Greater => {}
            }
        }
    }
    candidate.owner = Some(owner);
    candidate.linked_issue = Some(linked_issue);
    candidate.state = target_state;
    candidate.blocked_reason = None;
    candidate.failure_subcode = None;
    candidate.retry = None;
    candidate.resolver_snapshot = None;
    candidate.attempt = None;
    if let Some(pending_create_resolution) = candidate.pending_create_resolution.clone() {
        super::improvement_store::clear_pending_create_resolution(
            candidate,
            &pending_create_resolution,
        )?;
    }
    if public_binding_changed || stable_status_changed {
        candidate.owner_status_generation = candidate
            .owner_status_generation
            .checked_add(1)
            .ok_or_else(|| owner_projection_error("owner status generation overflow"))?;
    }
    candidate.updated_at = later_rfc3339(&candidate.updated_at, &source.last_seen)?;
    super::improvement::validate_candidate_lifecycle(candidate)?;
    Ok(true)
}

fn later_rfc3339(left: &str, right: &str) -> Result<String, gwt_github::SpecOpsError> {
    let left_value = chrono::DateTime::parse_from_rfc3339(left)
        .map_err(|_| owner_projection_error("candidate updated_at must be RFC3339"))?;
    let right_value = chrono::DateTime::parse_from_rfc3339(right)
        .map_err(|_| owner_projection_error("projection last_seen must be RFC3339"))?;
    Ok(if right_value > left_value {
        right.to_string()
    } else {
        left.to_string()
    })
}

fn owner_projection_error(message: &str) -> gwt_github::SpecOpsError {
    gwt_github::SpecOpsError::from(GitHubApiError::Unexpected(message.to_string()))
}

pub(super) trait SemanticOwnerAdvisor {
    fn owner_numbers(
        &self,
        candidate: &ImprovementCandidate,
        deadline: &ResolutionDeadline,
    ) -> Result<Vec<IssueNumber>, ()>;
}

#[derive(Debug, Default)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct NoSemanticOwnerAdvisor;

impl SemanticOwnerAdvisor for NoSemanticOwnerAdvisor {
    fn owner_numbers(
        &self,
        _candidate: &ImprovementCandidate,
        _deadline: &ResolutionDeadline,
    ) -> Result<Vec<IssueNumber>, ()> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OwnerPreflightOutcome {
    Active {
        owner: OwnerCandidate,
        comments: Vec<RepositoryComment>,
        corpus_generation: String,
    },
    Historical {
        owners: Vec<OwnerCandidate>,
        comments_by_owner: BTreeMap<IssueNumber, Vec<RepositoryComment>>,
        corpus_generation: String,
    },
    RegressionCreateAuthorized {
        authorization: ProvenRegressionAuthorization,
    },
    CreateAuthorized {
        authorization: ProvenZeroAuthorization,
    },
    DuplicateExactIssues {
        owners: Vec<OwnerCandidate>,
        comments_by_owner: BTreeMap<IssueNumber, Vec<RepositoryComment>>,
        corpus_generation: String,
    },
    RemoteOutcomeUnknown {
        failure_reason: BlockedReason,
        failure_subcode: Option<FailureSubcode>,
    },
    Blocked {
        reason: BlockedReason,
        failure_subcode: Option<FailureSubcode>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProvenZeroAuthorization {
    candidate_id: String,
    fingerprint: String,
    corpus_generation: String,
    payload: PublicIssuePayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProvenRegressionAuthorization {
    candidate_id: String,
    fingerprint: String,
    corpus_generation: String,
    historical_owner: OwnerCandidate,
    recurrence_occurrence_keys: Vec<String>,
    recurrence_proof_digest: String,
    payload: PublicIssuePayload,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct HistoricalResolutionMarker {
    merged_pr_number: u64,
    merge_commit_sha: String,
    verified_at: String,
    first_fixed_release: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrustedHistoricalResolution {
    marker: HistoricalResolutionMarker,
    comment_id: u64,
    comment_updated_at: String,
    author_login: String,
}

#[derive(Debug)]
struct CreatedOwnerMutation {
    readback: RepositoryIssue,
    input: CreateRepositoryIssue,
    fingerprint: String,
}

#[derive(Debug)]
enum OwnerInspection {
    Active {
        owner: OwnerCandidate,
        comments: Vec<RepositoryComment>,
        corpus_generation: String,
    },
    Historical {
        owners: Vec<OwnerCandidate>,
        comments_by_owner: BTreeMap<IssueNumber, Vec<RepositoryComment>>,
        corpus_generation: String,
    },
    Zero {
        corpus_generation: String,
        advisory_failed: bool,
    },
    DuplicateExactIssues {
        owners: Vec<OwnerCandidate>,
        comments_by_owner: BTreeMap<IssueNumber, Vec<RepositoryComment>>,
        corpus_generation: String,
    },
    Ambiguous {
        owner_candidates: Vec<OwnerCandidate>,
        corpus_generation: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub(super) struct OwnerResolutionFailure {
    pub(super) reason: BlockedReason,
    pub(super) failure_subcode: Option<FailureSubcode>,
    pub(super) remediation: &'static str,
}

#[derive(Debug)]
enum OwnerResolutionCommitError {
    PreSubmit {
        failure: OwnerResolutionFailure,
        source: gwt_github::SpecOpsError,
        clear_create_root: bool,
    },
    RemoteOutcomeUnknown {
        failure: OwnerResolutionFailure,
        source: gwt_github::SpecOpsError,
    },
    DurableStatus(gwt_github::SpecOpsError),
}

#[derive(Debug, Clone)]
struct ResolutionAttemptToken {
    candidate_id: String,
    attempt_id: String,
    ttl: chrono::Duration,
}

struct UnhandledResolutionSettlement {
    repo_root: PathBuf,
    token: ResolutionAttemptToken,
    resolution_deadline: ResolutionDeadline,
}

impl Drop for UnhandledResolutionSettlement {
    fn drop(&mut self) {
        let failure = if self
            .resolution_deadline
            .remaining("owner resolution fallback settlement")
            .is_err()
        {
            OwnerResolutionFailure {
                reason: BlockedReason::Timeout,
                failure_subcode: None,
                remediation: "RETRY_WITHIN_BUDGET",
            }
        } else {
            OwnerResolutionFailure {
                reason: BlockedReason::Store,
                failure_subcode: None,
                remediation: "RELOAD_CANDIDATE_STORE",
            }
        };
        let _ = settle_unhandled_resolution_failure(&self.repo_root, &self.token, failure);
    }
}

#[derive(Debug)]
enum ResolutionAttemptStart {
    Acquired {
        candidate: ImprovementCandidate,
        token: ResolutionAttemptToken,
        recovering_remote_unknown: bool,
        recovery_intent: ResolutionAttemptIntent,
        reconciliation_only: bool,
    },
}

#[derive(Debug)]
enum NewOwnerPostflight {
    Committed,
    Active {
        owner: OwnerCandidate,
        comments: Vec<RepositoryComment>,
    },
}

impl fmt::Display for OwnerResolutionCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreSubmit { source, .. }
            | Self::RemoteOutcomeUnknown { source, .. }
            | Self::DurableStatus(source) => source.fmt(formatter),
        }
    }
}

pub(super) fn owner_resolution_preflight<E, A>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    registry: &ContractRoutingRegistry,
    semantic_advisor: &A,
    deadline: &ResolutionDeadline,
) -> Result<OwnerPreflightOutcome, gwt_github::SpecOpsError>
where
    E: CliEnv,
    A: SemanticOwnerAdvisor,
{
    owner_resolution_preflight_inner(env, candidate, registry, semantic_advisor, deadline, true)
}

fn owner_resolution_preflight_deferred<E, A>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    registry: &ContractRoutingRegistry,
    semantic_advisor: &A,
    deadline: &ResolutionDeadline,
) -> Result<OwnerPreflightOutcome, gwt_github::SpecOpsError>
where
    E: CliEnv,
    A: SemanticOwnerAdvisor,
{
    owner_resolution_preflight_inner(env, candidate, registry, semantic_advisor, deadline, false)
}

fn owner_resolution_preflight_inner<E, A>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    registry: &ContractRoutingRegistry,
    semantic_advisor: &A,
    deadline: &ResolutionDeadline,
    publish_failure_status: bool,
) -> Result<OwnerPreflightOutcome, gwt_github::SpecOpsError>
where
    E: CliEnv,
    A: SemanticOwnerAdvisor,
{
    if deadline.remaining("collect owner privacy context").is_err() {
        let failure = OwnerResolutionFailure {
            reason: BlockedReason::Timeout,
            failure_subcode: None,
            remediation: "RETRY_OWNER_RESOLUTION",
        };
        block_owner_resolution(env, candidate, failure, None, publish_failure_status)?;
        return Ok(owner_failure_outcome(candidate, failure));
    }
    let canonical_source_scope_nonce = match env.improvement_source_scope_nonce() {
        Ok(nonce) => nonce,
        Err(_) => {
            let failure = OwnerResolutionFailure {
                reason: BlockedReason::Store,
                failure_subcode: None,
                remediation: "REPAIR_CANDIDATE_STORE",
            };
            block_owner_resolution(env, candidate, failure, None, publish_failure_status)?;
            return Ok(owner_failure_outcome(candidate, failure));
        }
    };
    let repo_path = env.repo_path().to_path_buf();
    let public_context = PublicMutationContext::for_repo_with_deadline(&repo_path, deadline);
    if deadline.remaining("collect owner privacy context").is_err() {
        let failure = OwnerResolutionFailure {
            reason: BlockedReason::Timeout,
            failure_subcode: None,
            remediation: "RETRY_OWNER_RESOLUTION",
        };
        block_owner_resolution(env, candidate, failure, None, publish_failure_status)?;
        return Ok(owner_failure_outcome(candidate, failure));
    }
    if let Some(failure) = candidate_preflight_failure(
        candidate,
        &repo_path,
        &canonical_source_scope_nonce,
        &public_context,
    ) {
        block_owner_resolution(env, candidate, failure, None, publish_failure_status)?;
        return Ok(owner_failure_outcome(candidate, failure));
    }

    let inspection = match env.improvement_owner_client(deadline) {
        Ok(client) => inspect_owner_corpus(client, candidate, registry, semantic_advisor, deadline),
        Err(error) => Err(owner_failure_from_api(&error)),
    };

    match inspection {
        Ok(OwnerInspection::Active {
            owner,
            comments,
            corpus_generation,
        }) => Ok(OwnerPreflightOutcome::Active {
            owner,
            comments,
            corpus_generation,
        }),
        Ok(OwnerInspection::Historical {
            owners,
            comments_by_owner,
            corpus_generation,
        }) => {
            if !candidate
                .distinct_occurrences
                .iter()
                .any(|occurrence| occurrence.recurrence.is_some())
            {
                return Ok(OwnerPreflightOutcome::Historical {
                    owners,
                    comments_by_owner,
                    corpus_generation,
                });
            }
            let authorization = match env.improvement_owner_client(deadline) {
                Ok(client) => authorize_historical_regression(
                    client,
                    candidate,
                    &owners,
                    &comments_by_owner,
                    &corpus_generation,
                    &public_context,
                    deadline,
                ),
                Err(error) => Err(owner_failure_from_api(&error)),
            };
            match authorization {
                Ok(authorization) => {
                    Ok(OwnerPreflightOutcome::RegressionCreateAuthorized { authorization })
                }
                Err(failure) => {
                    block_owner_resolution(env, candidate, failure, None, publish_failure_status)?;
                    Ok(owner_failure_outcome(candidate, failure))
                }
            }
        }
        Ok(OwnerInspection::DuplicateExactIssues {
            owners,
            comments_by_owner,
            corpus_generation,
        }) => Ok(OwnerPreflightOutcome::DuplicateExactIssues {
            owners,
            comments_by_owner,
            corpus_generation,
        }),
        Ok(OwnerInspection::Zero {
            corpus_generation,
            advisory_failed,
        }) => {
            if advisory_failed {
                post_improvement_board_status(
                    env,
                    format!(
                        "Current state: Improvement Candidate {} authoritative owner search completed with advisory diagnostic ADVISORY_INDEX_UNAVAILABLE.\n\nReason: The local semantic advisor is non-authoritative.\n\nNext: Continue from the complete authoritative corpus.",
                        candidate.id
                    ),
                )?;
            }
            let dispositions = registry.matching_dispositions(candidate);
            match dispositions.as_slice() {
                [ContractRouteDisposition::ImplementationGap] => {
                    let fingerprint = candidate.fingerprint.clone().ok_or_else(|| {
                        owner_projection_error("candidate fingerprint is missing")
                    })?;
                    let payload = render_public_issue_payload(candidate, &public_context)
                        .map_err(privacy_as_owner_spec_error)?;
                    Ok(OwnerPreflightOutcome::CreateAuthorized {
                        authorization: ProvenZeroAuthorization {
                            candidate_id: candidate.id.clone(),
                            fingerprint,
                            corpus_generation,
                            payload,
                        },
                    })
                }
                [ContractRouteDisposition::SpecGap | ContractRouteDisposition::SpecAmbiguous] => {
                    let failure = OwnerResolutionFailure {
                        reason: BlockedReason::Ambiguity,
                        failure_subcode: None,
                        remediation: "CLARIFY_CONTRACT_ROUTING",
                    };
                    block_owner_resolution(env, candidate, failure, None, publish_failure_status)?;
                    Ok(owner_failure_outcome(candidate, failure))
                }
                _ => {
                    let failure = OwnerResolutionFailure {
                        reason: BlockedReason::Routing,
                        failure_subcode: None,
                        remediation: "REFRESH_CONTRACT_ROUTING",
                    };
                    block_owner_resolution(env, candidate, failure, None, publish_failure_status)?;
                    Ok(owner_failure_outcome(candidate, failure))
                }
            }
        }
        Ok(OwnerInspection::Ambiguous {
            owner_candidates,
            corpus_generation,
        }) => {
            let snapshot = ResolverSnapshot::new(corpus_generation, owner_candidates)?;
            let failure = OwnerResolutionFailure {
                reason: BlockedReason::Ambiguity,
                failure_subcode: None,
                remediation: "SELECT_VERIFIED_OWNER",
            };
            block_owner_resolution(
                env,
                candidate,
                failure,
                Some(snapshot),
                publish_failure_status,
            )?;
            Ok(owner_failure_outcome(candidate, failure))
        }
        Err(failure) => {
            block_owner_resolution(env, candidate, failure, None, publish_failure_status)?;
            Ok(owner_failure_outcome(candidate, failure))
        }
    }
}

fn authorize_historical_regression<C: OwnerRepositoryClient + ?Sized>(
    client: &C,
    candidate: &ImprovementCandidate,
    owners: &[OwnerCandidate],
    comments_by_owner: &BTreeMap<IssueNumber, Vec<RepositoryComment>>,
    corpus_generation: &str,
    public_context: &PublicMutationContext,
    deadline: &ResolutionDeadline,
) -> Result<ProvenRegressionAuthorization, OwnerResolutionFailure> {
    let ambiguous = || OwnerResolutionFailure {
        reason: BlockedReason::Ambiguity,
        failure_subcode: None,
        remediation: "VERIFY_HISTORICAL_RECURRENCE",
    };
    let durable_owner = candidate.owner.as_ref().ok_or_else(ambiguous)?;
    let fingerprint = candidate.fingerprint.as_deref().ok_or_else(ambiguous)?;
    let mut current_generation_owners = owners
        .iter()
        .filter(|owner| owner.number == durable_owner.number);
    let historical_owner = current_generation_owners.next().ok_or_else(ambiguous)?;
    if current_generation_owners.next().is_some() {
        return Err(ambiguous());
    }
    if corpus_generation.trim().is_empty()
        || historical_owner.active
        || historical_owner.selectable
        || historical_owner.kind != OwnerKind::Issue
        || historical_owner.match_basis != OwnerMatchBasis::Fingerprint
        || durable_owner.number != historical_owner.number
        || durable_owner.kind != historical_owner.kind
        || durable_owner.fingerprint != fingerprint
    {
        return Err(ambiguous());
    }

    let comments = comments_by_owner
        .get(&IssueNumber(historical_owner.number))
        .ok_or_else(ambiguous)?;
    let resolution = trusted_historical_resolution(comments).ok_or_else(ambiguous)?;
    if resolution.marker.merged_pr_number == 0
        || !is_full_git_commit_oid(&resolution.marker.merge_commit_sha)
        || resolution.marker.first_fixed_release.trim().is_empty()
    {
        return Err(ambiguous());
    }

    let repository = RepositoryIdentity::gwt_upstream();
    let merged_pr = client
        .fetch_merged_pull_request(
            &repository,
            IssueNumber(resolution.marker.merged_pr_number),
            deadline,
        )
        .map_err(|error| owner_failure_from_api(&error))?
        .ok_or_else(ambiguous)?;
    if merged_pr.number != IssueNumber(resolution.marker.merged_pr_number)
        || merged_pr.merge_commit_sha != resolution.marker.merge_commit_sha
    {
        return Err(ambiguous());
    }
    let release = client
        .fetch_release_by_tag(
            &repository,
            &resolution.marker.first_fixed_release,
            deadline,
        )
        .map_err(|error| owner_failure_from_api(&error))?
        .ok_or_else(ambiguous)?;
    if release.tag_name != resolution.marker.first_fixed_release {
        return Err(ambiguous());
    }
    let release_ref = format!("refs/tags/{}", release.tag_name);
    let release_contains_merge = client
        .compare_commits(
            &repository,
            &resolution.marker.merge_commit_sha,
            &release_ref,
            deadline,
        )
        .map_err(|error| owner_failure_from_api(&error))?;
    if !matches!(
        release_contains_merge.status,
        CommitComparisonStatus::Ahead | CommitComparisonStatus::Identical
    ) {
        return Err(ambiguous());
    }
    let release_commit_sha = release_contains_merge.head_commit_sha.clone();

    let merged_at = parse_history_timestamp(&merged_pr.merged_at).ok_or_else(ambiguous)?;
    let published_at = parse_history_timestamp(&release.published_at).ok_or_else(ambiguous)?;
    let verified_at =
        parse_history_timestamp(&resolution.marker.verified_at).ok_or_else(ambiguous)?;
    let comment_updated_at =
        parse_history_timestamp(&resolution.comment_updated_at).ok_or_else(ambiguous)?;
    let fixed_at = std::cmp::max(merged_at, published_at);
    if merged_at > published_at || verified_at < fixed_at || verified_at > comment_updated_at {
        return Err(ambiguous());
    }
    let fixed_version = parse_release_version(&release.tag_name).ok_or_else(ambiguous)?;

    let resolution_cutoff = std::cmp::max(fixed_at, verified_at);
    let mut recurrence_occurrences = Vec::new();
    for occurrence in candidate
        .distinct_occurrences
        .iter()
        .filter(|occurrence| occurrence.recurrence.is_some() && occurrence.qualifies_unattended)
    {
        let recurrence = occurrence.recurrence.as_ref().expect("filtered recurrence");
        let observed_at = parse_history_timestamp(&recurrence.observed_at).ok_or_else(ambiguous)?;
        let captured_at = parse_history_timestamp(&occurrence.captured_at).ok_or_else(ambiguous)?;
        if observed_at > captured_at {
            return Err(ambiguous());
        }
        if observed_at > resolution_cutoff {
            recurrence_occurrences.push(occurrence);
        }
    }
    recurrence_occurrences.sort_by(|left, right| left.opaque_key.cmp(&right.opaque_key));
    if recurrence_occurrences.is_empty() {
        return Err(ambiguous());
    }
    for occurrence in &recurrence_occurrences {
        let recurrence = occurrence.recurrence.as_ref().expect("filtered recurrence");
        if recurrence.installed_version.is_none() && recurrence.build_commit.is_none() {
            return Err(ambiguous());
        }
        if let Some(installed_version) = recurrence.installed_version.as_deref() {
            let installed_version =
                parse_release_version(installed_version).ok_or_else(ambiguous)?;
            if installed_version <= fixed_version {
                return Err(ambiguous());
            }
        }
        if let Some(build_commit) = recurrence.build_commit.as_deref() {
            if !is_full_git_commit_oid(build_commit) {
                return Err(ambiguous());
            }
            let comparison = client
                .compare_commits(&repository, &release_commit_sha, build_commit, deadline)
                .map_err(|error| owner_failure_from_api(&error))?;
            if comparison.status != CommitComparisonStatus::Ahead {
                return Err(ambiguous());
            }
        }
    }

    let recurrence_occurrence_keys = recurrence_occurrences
        .iter()
        .map(|occurrence| occurrence.opaque_key.clone())
        .collect::<Vec<_>>();
    let mut proof_fields = vec![
        historical_owner.number.to_string(),
        fingerprint.to_string(),
        resolution.comment_id.to_string(),
        resolution.comment_updated_at.clone(),
        resolution.author_login.clone(),
        resolution.marker.merged_pr_number.to_string(),
        resolution.marker.merge_commit_sha.clone(),
        merged_pr.merged_at.clone(),
        resolution.marker.verified_at.clone(),
        release.tag_name.clone(),
        release_commit_sha,
        release.published_at.clone(),
    ];
    for occurrence in &recurrence_occurrences {
        proof_fields.push(occurrence.opaque_key.clone());
        proof_fields.push(occurrence.evidence_digest.clone());
    }
    let proof_refs = proof_fields.iter().map(String::as_str).collect::<Vec<_>>();
    let recurrence_proof_digest = public_payload_digest(
        "gwt.improvement.historical-recurrence-proof.v1",
        &proof_refs,
    );
    let payload = render_regression_issue_payload(
        candidate,
        historical_owner.number,
        &recurrence_proof_digest,
        public_context,
    )
    .map_err(|_| OwnerResolutionFailure {
        reason: BlockedReason::Privacy,
        failure_subcode: None,
        remediation: "RECAPTURE_SAFE_TYPED_EVIDENCE",
    })?;
    Ok(ProvenRegressionAuthorization {
        candidate_id: candidate.id.clone(),
        fingerprint: fingerprint.to_string(),
        corpus_generation: corpus_generation.to_string(),
        historical_owner: historical_owner.clone(),
        recurrence_occurrence_keys,
        recurrence_proof_digest,
        payload,
    })
}

fn trusted_historical_resolution(
    comments: &[RepositoryComment],
) -> Option<TrustedHistoricalResolution> {
    let mut resolutions = Vec::new();
    for comment in comments {
        let marker_payloads = historical_resolution_marker_payloads(&comment.body)?;
        if marker_payloads.is_empty() {
            continue;
        }
        let author_login = comment
            .author_login
            .as_deref()
            .filter(|login| !login.trim().is_empty());
        let actor_is_trusted = matches!(
            comment.author_type,
            Some(RepositoryActorType::User | RepositoryActorType::EnterpriseUserAccount)
        ) && matches!(
            comment.author_association,
            Some(
                RepositoryAuthorAssociation::Owner
                    | RepositoryAuthorAssociation::Member
                    | RepositoryAuthorAssociation::Collaborator
            )
        );
        let author_login = author_login.filter(|_| actor_is_trusted)?;
        for marker_json in marker_payloads {
            let marker = serde_json::from_str::<HistoricalResolutionMarker>(&marker_json).ok()?;
            resolutions.push(TrustedHistoricalResolution {
                marker,
                comment_id: comment.id.0,
                comment_updated_at: comment.updated_at.0.clone(),
                author_login: author_login.to_string(),
            });
        }
    }
    let [resolution] = resolutions.as_slice() else {
        return None;
    };
    Some(resolution.clone())
}

fn historical_resolution_marker_payloads(body: &str) -> Option<Vec<String>> {
    const TOKEN: &str = "gwt-improvement-resolution:v1";
    const PREFIX: &str = "<!-- gwt-improvement-resolution:v1 ";
    const SUFFIX: &str = " -->";
    let mut code_fence: Option<(u8, usize)> = None;
    let mut payloads = Vec::new();
    for line in body.lines() {
        if let Some((delimiter, length, trailing_blank)) = markdown_fence_delimiter(line) {
            match code_fence {
                Some((open_delimiter, open_length))
                    if delimiter == open_delimiter && length >= open_length && trailing_blank =>
                {
                    code_fence = None;
                    continue;
                }
                None => {
                    code_fence = Some((delimiter, length));
                    continue;
                }
                _ => {}
            }
        }
        if code_fence.is_some() || !line.contains(TOKEN) {
            continue;
        }
        let payload = line.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
        if payload.trim() != payload || payload.is_empty() {
            return None;
        }
        payloads.push(payload.to_string());
    }
    Some(payloads)
}

fn parse_history_timestamp(value: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(value).ok()
}

fn parse_release_version(value: &str) -> Option<Version> {
    let normalized = value.strip_prefix('v').unwrap_or(value);
    if normalized.is_empty() || normalized.starts_with('v') {
        return None;
    }
    Version::parse(normalized).ok()
}

fn is_full_git_commit_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn render_regression_issue_payload(
    candidate: &ImprovementCandidate,
    historical_owner_number: u64,
    recurrence_proof_digest: &str,
    context: &PublicMutationContext,
) -> Result<PublicIssuePayload, PrivacyViolation> {
    if historical_owner_number == 0
        || !recurrence_proof_digest
            .strip_prefix("sha256:")
            .is_some_and(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    let mut payload = render_public_issue_payload(candidate, context)?;
    payload.body.push_str(&format!(
        "\n## Verified recurrence\n\n- Historical owner: #{historical_owner_number}\n- Recurrence proof: {recurrence_proof_digest}\n\n<!-- gwt:improvement-regression:v1 historical:{historical_owner_number} proof:{recurrence_proof_digest} -->\n"
    ));
    let evidence = candidate
        .typed_evidence
        .as_ref()
        .ok_or_else(|| PrivacyViolation::new(PrivacyViolationKind::InvalidTemplateField))?;
    let identity = typed_public_identity(candidate, evidence)?;
    validate_public_payload(
        &payload,
        &context.with_candidate(
            candidate,
            &[&identity.evidence_digest, &identity.fingerprint],
        ),
    )?;
    Ok(payload)
}

fn owner_failure_outcome(
    candidate: &ImprovementCandidate,
    failure: OwnerResolutionFailure,
) -> OwnerPreflightOutcome {
    if candidate.state == CandidateState::RemoteOutcomeUnknown {
        OwnerPreflightOutcome::RemoteOutcomeUnknown {
            failure_reason: failure.reason,
            failure_subcode: failure.failure_subcode,
        }
    } else {
        OwnerPreflightOutcome::Blocked {
            reason: failure.reason,
            failure_subcode: failure.failure_subcode,
        }
    }
}

fn candidate_preflight_failure(
    candidate: &ImprovementCandidate,
    repo_root: &Path,
    canonical_source_scope_nonce: &str,
    public_context: &PublicMutationContext,
) -> Option<OwnerResolutionFailure> {
    let Some(evidence) = candidate.typed_evidence.as_ref() else {
        return Some(OwnerResolutionFailure {
            reason: BlockedReason::Routing,
            failure_subcode: None,
            remediation: "CAPTURE_TYPED_EVIDENCE",
        });
    };
    let canonical_fingerprint = improvement_fingerprint(evidence);
    let identity_is_canonical =
        candidate.fingerprint.as_deref() == Some(canonical_fingerprint.as_str());
    let eligibility_is_valid = super::improvement::owner_eligibility_is_canonical(
        candidate,
        repo_root,
        canonical_source_scope_nonce,
    );
    if !identity_is_canonical || !eligibility_is_valid {
        return Some(OwnerResolutionFailure {
            reason: BlockedReason::Routing,
            failure_subcode: None,
            remediation: "CAPTURE_TYPED_EVIDENCE",
        });
    }

    let issue_payload_is_safe = render_public_issue_payload(candidate, public_context).is_ok();
    let occurrence_payloads_are_safe = candidate.distinct_occurrences.iter().all(|occurrence| {
        render_occurrence_comment_payload(candidate, &occurrence.opaque_key, public_context).is_ok()
    });
    if !issue_payload_is_safe || !occurrence_payloads_are_safe {
        return Some(OwnerResolutionFailure {
            reason: BlockedReason::Privacy,
            failure_subcode: None,
            remediation: "RECAPTURE_SAFE_TYPED_EVIDENCE",
        });
    }
    None
}

fn inspect_owner_corpus<C, A>(
    client: &C,
    candidate: &ImprovementCandidate,
    registry: &ContractRoutingRegistry,
    semantic_advisor: &A,
    deadline: &ResolutionDeadline,
) -> Result<OwnerInspection, OwnerResolutionFailure>
where
    C: OwnerRepositoryClient + ?Sized,
    A: SemanticOwnerAdvisor,
{
    let repository = RepositoryIdentity::gwt_upstream();
    let issue_collection = client
        .list_issues(&repository, deadline)
        .map_err(|error| owner_failure_from_api(&error))?;
    if issue_collection.items().is_empty() {
        return Err(OwnerResolutionFailure {
            reason: BlockedReason::Search,
            failure_subcode: Some(FailureSubcode::EmptyCorpus),
            remediation: "RETRY_OWNER_SEARCH",
        });
    }
    if issue_collection.generation().as_str().is_empty() {
        return Err(OwnerResolutionFailure {
            reason: BlockedReason::Search,
            failure_subcode: Some(FailureSubcode::PartialPage),
            remediation: "RETRY_OWNER_SEARCH",
        });
    }

    let fingerprint = candidate
        .fingerprint
        .as_deref()
        .filter(|value| fingerprint_value_re().is_match(value))
        .ok_or(OwnerResolutionFailure {
            reason: BlockedReason::Routing,
            failure_subcode: None,
            remediation: "CAPTURE_TYPED_EVIDENCE",
        })?;
    let issue_generation = issue_collection.generation().as_str().to_string();
    let issue_by_number = issue_collection
        .items()
        .iter()
        .map(|issue| (issue.number, issue))
        .collect::<BTreeMap<_, _>>();
    let mut matches = BTreeMap::<IssueNumber, OwnerCandidate>::new();

    for issue in issue_collection.items() {
        let markers = exact_fingerprint_markers(&issue.body);
        if markers.iter().any(|marker| marker == fingerprint) {
            let mut owner = owner_candidate(issue, OwnerMatchBasis::Fingerprint);
            owner.selectable = owner.active;
            matches.insert(issue.number, owner);
        }
    }

    for owner_number in registry.matching_owner_numbers(candidate) {
        let Some(issue) = issue_by_number.get(&owner_number) else {
            return Err(OwnerResolutionFailure {
                reason: BlockedReason::Ambiguity,
                failure_subcode: None,
                remediation: "REFRESH_CONTRACT_ROUTING",
            });
        };
        matches
            .entry(owner_number)
            .or_insert_with(|| owner_candidate(issue, OwnerMatchBasis::Contract));
    }

    let active_owner_numbers = matches
        .iter()
        .filter_map(|(number, owner)| owner.active.then_some(*number))
        .collect::<Vec<_>>();
    let duplicate_exact_issues = active_owner_numbers.len() > 1
        && active_owner_numbers.iter().all(|number| {
            matches.get(number).is_some_and(|owner| {
                owner.kind == OwnerKind::Issue
                    && owner.match_basis == OwnerMatchBasis::Fingerprint
                    && owner.selectable
            })
        });
    let comment_owner_numbers = if active_owner_numbers.len() == 1 || duplicate_exact_issues {
        active_owner_numbers.clone()
    } else if active_owner_numbers.is_empty() {
        matches.keys().copied().collect()
    } else {
        Vec::new()
    };
    let mut comments_by_owner = BTreeMap::new();
    let mut comment_generations = BTreeMap::new();
    for owner_number in comment_owner_numbers {
        let comments = client
            .list_comments(&repository, owner_number, deadline)
            .map_err(|error| owner_failure_from_api(&error))?;
        if comments.generation().as_str().is_empty() {
            return Err(OwnerResolutionFailure {
                reason: BlockedReason::Search,
                failure_subcode: Some(FailureSubcode::PartialPage),
                remediation: "RETRY_OWNER_SEARCH",
            });
        }
        comment_generations.insert(owner_number, comments.generation().as_str().to_string());
        comments_by_owner.insert(owner_number, comments.into_items());
    }

    let semantic_result = if matches.is_empty() {
        deadline
            .remaining("semantic owner advisor")
            .map_err(|_| OwnerResolutionFailure {
                reason: BlockedReason::Timeout,
                failure_subcode: None,
                remediation: "RETRY_WITHIN_BUDGET",
            })?;
        let result = semantic_advisor.owner_numbers(candidate, deadline);
        deadline
            .remaining("semantic owner advisor")
            .map_err(|_| OwnerResolutionFailure {
                reason: BlockedReason::Timeout,
                failure_subcode: None,
                remediation: "RETRY_WITHIN_BUDGET",
            })?;
        Some(result)
    } else {
        None
    };

    let final_issue_collection = client
        .list_issues(&repository, deadline)
        .map_err(|error| owner_failure_from_api(&error))?;
    if final_issue_collection.generation().as_str().is_empty()
        || final_issue_collection.generation() != issue_collection.generation()
    {
        return Err(OwnerResolutionFailure {
            reason: BlockedReason::Search,
            failure_subcode: Some(FailureSubcode::PartialPage),
            remediation: "RETRY_OWNER_SEARCH",
        });
    }
    let mut generation_fields = vec![issue_generation];
    for (owner_number, generation) in &comment_generations {
        generation_fields.push(format!("{}:{}", owner_number.0, generation));
    }
    let corpus_generation = combined_corpus_generation(generation_fields);

    if !matches.is_empty() {
        let owners = matches.into_values().collect::<Vec<_>>();
        let mut active_owners = owners
            .iter()
            .filter(|owner| owner.active)
            .cloned()
            .collect::<Vec<_>>();
        if active_owners.len() > 1 {
            if duplicate_exact_issues {
                active_owners.sort_by_key(|owner| owner.number);
                return Ok(OwnerInspection::DuplicateExactIssues {
                    owners: active_owners,
                    comments_by_owner,
                    corpus_generation,
                });
            }
            return Ok(OwnerInspection::Ambiguous {
                owner_candidates: active_owners,
                corpus_generation,
            });
        }
        if let Some(owner) = active_owners.pop() {
            let owner_number = IssueNumber(owner.number);
            return Ok(OwnerInspection::Active {
                owner,
                comments: comments_by_owner.remove(&owner_number).unwrap_or_default(),
                corpus_generation,
            });
        }
        return Ok(OwnerInspection::Historical {
            owners,
            comments_by_owner,
            corpus_generation,
        });
    }

    let semantic_result = semantic_result.expect("zero-owner inspection runs semantic advice");
    if let Ok(semantic_numbers) = &semantic_result {
        let semantic_candidates = semantic_numbers
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|number| issue_by_number.get(&number))
            .map(|issue| {
                let mut owner = owner_candidate(issue, OwnerMatchBasis::Semantic);
                owner.selectable = false;
                owner
            })
            .collect::<Vec<_>>();
        if !semantic_candidates.is_empty() {
            return Ok(OwnerInspection::Ambiguous {
                owner_candidates: semantic_candidates,
                corpus_generation,
            });
        }
    }

    Ok(OwnerInspection::Zero {
        corpus_generation,
        advisory_failed: semantic_result.is_err(),
    })
}

fn verify_reconciliation_cleanup<E: CliEnv>(
    env: &mut E,
    candidate: &ImprovementCandidate,
    canonical_owner_number: u64,
    deadline: &ResolutionDeadline,
) -> Result<bool, gwt_github::SpecOpsError> {
    if !candidate.reconciliation_required {
        return Ok(false);
    }
    let known = &candidate.reconciliation_owner_numbers;
    if known.len() < 2 || known.first().copied() != Some(canonical_owner_number) {
        return Err(owner_projection_error(
            "reconciliation canonical owner does not match the durable lowest owner set",
        ));
    }
    let fingerprint = candidate
        .fingerprint
        .as_deref()
        .ok_or_else(|| owner_projection_error("candidate fingerprint is missing"))?;
    let repository = RepositoryIdentity::gwt_upstream();
    let client = env
        .improvement_owner_client(deadline)
        .map_err(gwt_github::SpecOpsError::from)?;
    for duplicate_number in known.iter().copied().skip(1) {
        let readback = client
            .fetch_issue(&repository, IssueNumber(duplicate_number), deadline)
            .map_err(gwt_github::SpecOpsError::from)?;
        if readback.repository != repository
            || readback.number != IssueNumber(duplicate_number)
            || readback.kind != RepositoryIssueKind::Plain
            || readback.state != IssueState::Closed
            || !exact_fingerprint_markers(&readback.body)
                .iter()
                .any(|marker| marker == fingerprint)
        {
            return Err(owner_projection_error(
                "known duplicate owner cleanup is not authoritatively verified",
            ));
        }
        let comments = client
            .list_comments(&repository, IssueNumber(duplicate_number), deadline)
            .map_err(gwt_github::SpecOpsError::from)?;
        if !comments.items().iter().any(|comment| {
            comment_matches_reconciliation(
                comment,
                canonical_owner_number,
                duplicate_number,
                fingerprint,
            )
        }) {
            return Err(owner_projection_error(
                "known duplicate owner reconciliation comment is not authoritatively verified",
            ));
        }
    }
    Ok(true)
}

fn commit_active_owner_resolution<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    owner: &OwnerCandidate,
    comments: &[RepositoryComment],
    deadline: &ResolutionDeadline,
) -> Result<(), OwnerResolutionCommitError> {
    commit_active_owner_resolution_with_audit(env, candidate, owner, comments, deadline, None, None)
}

fn commit_active_owner_resolution_for_attempt<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    owner: &OwnerCandidate,
    comments: &[RepositoryComment],
    deadline: &ResolutionDeadline,
) -> Result<(), OwnerResolutionCommitError> {
    commit_active_owner_resolution_with_audit(
        env,
        candidate,
        owner,
        comments,
        deadline,
        Some(token),
        None,
    )
}

#[derive(Clone, Copy)]
struct ManualOwnerSelectionAudit<'a> {
    token: &'a ResolutionAttemptToken,
    snapshot: &'a ResolverSnapshot,
}

fn commit_active_owner_resolution_with_audit<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    owner: &OwnerCandidate,
    comments: &[RepositoryComment],
    deadline: &ResolutionDeadline,
    attempt_token: Option<&ResolutionAttemptToken>,
    manual_audit: Option<ManualOwnerSelectionAudit<'_>>,
) -> Result<(), OwnerResolutionCommitError> {
    if !owner.active || !owner.selectable || owner.match_basis == OwnerMatchBasis::Semantic {
        return Err(pre_submit_commit_error(
            BlockedReason::Routing,
            "REFRESH_CONTRACT_ROUTING",
            owner_projection_error(
                "active owner resolution requires a selectable authoritative owner",
            ),
        ));
    }
    let repo_root = env.repo_path().to_path_buf();
    ensure_candidate_attempt_current(&repo_root, candidate).map_err(|error| {
        pre_submit_commit_error(BlockedReason::Store, "RELOAD_CANDIDATE_STORE", error)
    })?;
    let reconciliation_cleanup_verified =
        verify_reconciliation_cleanup(env, candidate, owner.number, deadline).map_err(|error| {
            pre_submit_commit_error(
                BlockedReason::Reconciliation,
                "RECONCILE_DUPLICATE_OWNERS",
                error,
            )
        })?;
    let fingerprint = candidate.fingerprint.clone().ok_or_else(|| {
        pre_submit_commit_error(
            BlockedReason::Routing,
            "CAPTURE_TYPED_EVIDENCE",
            owner_projection_error("candidate fingerprint is missing"),
        )
    })?;
    let public_context = PublicMutationContext::for_repo_with_deadline(&repo_root, deadline);
    let repository = RepositoryIdentity::gwt_upstream();
    let occurrences = candidate.distinct_occurrences.clone();
    if occurrences.is_empty()
        || !occurrences
            .iter()
            .any(|occurrence| occurrence.qualifies_unattended)
    {
        return Err(pre_submit_commit_error(
            BlockedReason::Routing,
            "CAPTURE_TYPED_EVIDENCE",
            owner_projection_error("active owner resolution requires a qualifying occurrence"),
        ));
    }
    validate_source_occurrence_snapshot(&repo_root, candidate, &occurrences).map_err(|error| {
        pre_submit_commit_error(BlockedReason::Store, "RELOAD_CANDIDATE_STORE", error)
    })?;

    let mut remote_mutation_seen = false;
    let projection_commits = {
        let client = env
            .improvement_owner_client(deadline)
            .map_err(pre_submit_commit_error_from_api)?;
        let initial_readback = client
            .fetch_issue(&repository, IssueNumber(owner.number), deadline)
            .map_err(|error| active_api_commit_error(error, remote_mutation_seen))?;
        validate_active_owner_readback(owner, &fingerprint, &initial_readback).map_err(
            |error| {
                active_spec_commit_error(
                    error,
                    remote_mutation_seen,
                    BlockedReason::Readback,
                    "REFRESH_OWNER_CORPUS",
                )
            },
        )?;
        let verified_owner = durable_owner_from_readback(&initial_readback, &fingerprint);
        validate_projection_owner_for_candidate(candidate, &verified_owner, &public_context)
            .map_err(|error| {
                active_spec_commit_error(
                    error,
                    remote_mutation_seen,
                    BlockedReason::LocalCommit,
                    "REPAIR_OWNER_PROJECTION",
                )
            })?;

        let payloads = occurrences
            .iter()
            .map(|occurrence| {
                render_occurrence_comment_payload(
                    candidate,
                    &occurrence.opaque_key,
                    &public_context,
                )
                .map_err(|error| {
                    owner_projection_error(&format!(
                        "active owner occurrence privacy validation failed: {error}"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                active_spec_commit_error(
                    error,
                    remote_mutation_seen,
                    BlockedReason::Privacy,
                    "RECAPTURE_SAFE_TYPED_EVIDENCE",
                )
            })?;
        let mut projection_commits = occurrences
            .iter()
            .map(|occurrence| {
                prepare_owner_projection_commit_with_context(
                    &repo_root,
                    &ReadbackVerifiedOwnerBinding {
                        candidate_id: candidate.id.clone(),
                        owner: verified_owner.clone(),
                        occurrence_key: occurrence.opaque_key.clone(),
                        resolution_status: CandidateState::Linked,
                        last_seen: occurrence.captured_at.clone(),
                    },
                    &public_context,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                active_spec_commit_error(
                    error,
                    remote_mutation_seen,
                    BlockedReason::LocalCommit,
                    "REPAIR_OWNER_PROJECTION",
                )
            })?;
        validate_source_occurrence_snapshot(&repo_root, candidate, &occurrences).map_err(
            |error| {
                active_spec_commit_error(
                    error,
                    remote_mutation_seen,
                    BlockedReason::Store,
                    "RELOAD_CANDIDATE_STORE",
                )
            },
        )?;

        for (occurrence, payload) in occurrences.iter().zip(&payloads) {
            if comments.iter().any(|comment| {
                comment_matches_occurrence(comment, &occurrence.opaque_key, &fingerprint)
            }) {
                continue;
            }
            let intent = ResolutionAttemptIntent::OccurrenceComments {
                owner_number: owner.number,
                occurrence_keys: vec![occurrence.opaque_key.clone()],
                public_payload_digest: public_payload_digest(
                    "gwt.improvement.owner-comment-intent.v1",
                    &[&occurrence.opaque_key, &payload.body],
                ),
            };
            if let Some(token) = attempt_token {
                mark_resolution_attempt_submitted(&repo_root, candidate, token, intent.clone())?;
            }
            ensure_candidate_attempt_current(&repo_root, candidate).map_err(|error| {
                active_spec_commit_error(
                    error,
                    remote_mutation_seen,
                    BlockedReason::Store,
                    "RELOAD_CANDIDATE_STORE",
                )
            })?;
            let created = match client.create_owner_comment(
                &repository,
                IssueNumber(owner.number),
                &payload.body,
                deadline,
            ) {
                Ok(created) => {
                    remote_mutation_seen = true;
                    created
                }
                Err(error) => {
                    return Err(active_mutation_commit_error(error, remote_mutation_seen));
                }
            };
            if created.body != payload.body {
                return Err(active_spec_commit_error(
                    owner_projection_error(
                        "active owner occurrence comment readback did not match its payload",
                    ),
                    remote_mutation_seen,
                    BlockedReason::Readback,
                    "REFRESH_OWNER_CORPUS",
                ));
            }
            let readback_comments = client
                .list_comments(&repository, IssueNumber(owner.number), deadline)
                .map_err(|error| active_api_commit_error(error, remote_mutation_seen))?;
            if !readback_comments.items().iter().any(|comment| {
                comment_matches_occurrence(comment, &occurrence.opaque_key, &fingerprint)
            }) {
                return Err(active_spec_commit_error(
                    owner_projection_error(
                        "active owner occurrence comment was absent from step readback",
                    ),
                    remote_mutation_seen,
                    BlockedReason::Readback,
                    "REFRESH_OWNER_CORPUS",
                ));
            }
            if let Some(token) = attempt_token {
                complete_occurrence_comment_step(&repo_root, candidate, token, &intent)?;
            }
            remote_mutation_seen = false;
        }

        let final_comments = client
            .list_comments(&repository, IssueNumber(owner.number), deadline)
            .map_err(|error| active_api_commit_error(error, remote_mutation_seen))?;
        let mut comment_ids_by_occurrence = BTreeMap::new();
        for occurrence in &occurrences {
            let mut comment_ids = final_comments
                .items()
                .iter()
                .filter(|comment| {
                    comment_matches_occurrence(comment, &occurrence.opaque_key, &fingerprint)
                })
                .map(|comment| comment.id.0)
                .collect::<Vec<_>>();
            comment_ids.sort_unstable();
            comment_ids.dedup();
            if comment_ids.is_empty() {
                return Err(active_spec_commit_error(
                    owner_projection_error(
                        "active owner occurrence comment was absent from final readback",
                    ),
                    remote_mutation_seen,
                    BlockedReason::Readback,
                    "REFRESH_OWNER_CORPUS",
                ));
            }
            comment_ids_by_occurrence.insert(occurrence.opaque_key.clone(), comment_ids);
        }

        let final_readback = client
            .fetch_issue(&repository, IssueNumber(owner.number), deadline)
            .map_err(|error| active_api_commit_error(error, remote_mutation_seen))?;
        validate_active_owner_readback(owner, &fingerprint, &final_readback).map_err(|error| {
            active_spec_commit_error(
                error,
                remote_mutation_seen,
                BlockedReason::Readback,
                "REFRESH_OWNER_CORPUS",
            )
        })?;
        let final_owner = durable_owner_from_readback(&final_readback, &fingerprint);
        validate_projection_owner_for_candidate(candidate, &final_owner, &public_context).map_err(
            |error| {
                active_spec_commit_error(
                    error,
                    remote_mutation_seen,
                    BlockedReason::LocalCommit,
                    "REPAIR_OWNER_PROJECTION",
                )
            },
        )?;
        let projected_owner = owner_projection_owner(&final_owner);
        for commit in &mut projection_commits {
            commit.owner = projected_owner.clone();
            commit.occurrence.comment_audit = super::improvement_store::StoredCommentAudit {
                completeness: super::improvement_store::StoredCommentAuditCompleteness::Complete,
                physical_comments: comment_ids_by_occurrence
                    .remove(&commit.occurrence.opaque_key)
                    .expect("every prevalidated occurrence has comment readback")
                    .into_iter()
                    .map(|comment_id| super::improvement_store::StoredCommentRef {
                        owner_number: final_owner.number,
                        comment_id,
                    })
                    .collect(),
            };
        }
        projection_commits
    };

    if let Some(manual_audit) = manual_audit {
        let audited = append_manual_owner_selection_audit(
            &repo_root,
            candidate,
            manual_audit.token,
            owner.number,
            manual_audit.snapshot,
        )
        .map_err(|error| {
            active_spec_commit_error(
                error,
                remote_mutation_seen,
                BlockedReason::LocalCommit,
                "REPAIR_OWNER_PROJECTION",
            )
        })?;
        *candidate = audited;
    }

    let repaired = commit_owner_projection_and_source_success(
        &repo_root,
        candidate,
        projection_commits,
        owner.number,
        reconciliation_cleanup_verified,
    )
    .map_err(|error| {
        active_spec_commit_error(
            error,
            remote_mutation_seen,
            BlockedReason::LocalCommit,
            "REPAIR_OWNER_PROJECTION",
        )
    })?;
    if !matches!(
        repaired.state,
        CandidateState::Linked | CandidateState::Created
    ) || repaired
        .owner
        .as_ref()
        .is_none_or(|repaired_owner| repaired_owner.number != owner.number)
    {
        return Err(active_spec_commit_error(
            owner_projection_error(
                "source candidate projection coverage did not reach a successful owner state",
            ),
            remote_mutation_seen,
            BlockedReason::LocalCommit,
            "REPAIR_OWNER_PROJECTION",
        ));
    }
    *candidate = repaired;
    deliver_pending_owner_status(env, candidate).map_err(OwnerResolutionCommitError::DurableStatus)
}

fn commit_new_owner_resolution<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    authorization: &ProvenZeroAuthorization,
    deadline: &ResolutionDeadline,
) -> Result<(), OwnerResolutionCommitError> {
    let created = create_new_owner_mutation(env, candidate, authorization, deadline)?;
    commit_created_owner_readback(
        env,
        candidate,
        &created.readback,
        &created.input,
        &created.fingerprint,
        deadline,
    )
}

fn create_new_owner_mutation<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    authorization: &ProvenZeroAuthorization,
    deadline: &ResolutionDeadline,
) -> Result<CreatedOwnerMutation, OwnerResolutionCommitError> {
    let repo_root = env.repo_path().to_path_buf();
    let fingerprint = candidate.fingerprint.clone().ok_or_else(|| {
        pre_submit_commit_error(
            BlockedReason::Routing,
            "CAPTURE_TYPED_EVIDENCE",
            owner_projection_error("candidate fingerprint is missing"),
        )
    })?;
    if candidate.id != authorization.candidate_id
        || fingerprint != authorization.fingerprint
        || authorization.corpus_generation.trim().is_empty()
        || candidate.state != CandidateState::OwnerResolving
    {
        return Err(pre_submit_commit_error(
            BlockedReason::Routing,
            "REFRESH_CONTRACT_ROUTING",
            owner_projection_error("proven-zero authorization does not match the candidate"),
        ));
    }
    let occurrences = candidate.distinct_occurrences.clone();
    if occurrences.is_empty()
        || !occurrences
            .iter()
            .any(|occurrence| occurrence.qualifies_unattended)
    {
        return Err(pre_submit_commit_error(
            BlockedReason::Routing,
            "CAPTURE_TYPED_EVIDENCE",
            owner_projection_error("new owner resolution requires a qualifying occurrence"),
        ));
    }

    let public_context = PublicMutationContext::for_repo_with_deadline(&repo_root, deadline);
    let rendered = render_public_issue_payload(candidate, &public_context).map_err(|error| {
        pre_submit_commit_error(
            BlockedReason::Privacy,
            "RECAPTURE_SAFE_TYPED_EVIDENCE",
            privacy_as_owner_spec_error(error),
        )
    })?;
    if rendered != authorization.payload {
        return Err(pre_submit_commit_error(
            BlockedReason::Privacy,
            "RECAPTURE_SAFE_TYPED_EVIDENCE",
            owner_projection_error("authorized public payload changed before owner creation"),
        ));
    }
    validate_source_occurrence_snapshot(&repo_root, candidate, &occurrences).map_err(|error| {
        pre_submit_commit_error(BlockedReason::Store, "RELOAD_CANDIDATE_STORE", error)
    })?;

    let repository = RepositoryIdentity::gwt_upstream();
    let input = CreateRepositoryIssue {
        title: authorization.payload.title.clone(),
        body: authorization.payload.body.clone(),
        labels: Vec::new(),
    };
    let readback = {
        let client = env
            .improvement_owner_client(deadline)
            .map_err(pre_submit_commit_error_from_api)?;
        client
            .create_owner_issue(&repository, &input, deadline)
            .map_err(commit_error_from_mutation)?
    };
    Ok(CreatedOwnerMutation {
        readback,
        input,
        fingerprint,
    })
}

fn create_regression_owner_mutation<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    authorization: &ProvenRegressionAuthorization,
    deadline: &ResolutionDeadline,
) -> Result<CreatedOwnerMutation, OwnerResolutionCommitError> {
    let repo_root = env.repo_path().to_path_buf();
    let fingerprint = candidate.fingerprint.clone().ok_or_else(|| {
        pre_submit_commit_error(
            BlockedReason::Routing,
            "CAPTURE_TYPED_EVIDENCE",
            owner_projection_error("candidate fingerprint is missing"),
        )
    })?;
    let durable_owner_matches = candidate.owner.as_ref().is_some_and(|owner| {
        owner.number == authorization.historical_owner.number && owner.fingerprint == fingerprint
    });
    if candidate.id != authorization.candidate_id
        || fingerprint != authorization.fingerprint
        || authorization.corpus_generation.trim().is_empty()
        || candidate.state != CandidateState::OwnerResolving
        || authorization.historical_owner.active
        || authorization.historical_owner.selectable
        || !durable_owner_matches
    {
        return Err(pre_submit_commit_error(
            BlockedReason::Ambiguity,
            "VERIFY_HISTORICAL_RECURRENCE",
            owner_projection_error("regression authorization does not match the candidate"),
        ));
    }
    validate_regression_occurrence_keys(candidate, &authorization.recurrence_occurrence_keys)?;

    let public_context = PublicMutationContext::for_repo_with_deadline(&repo_root, deadline);
    let rendered = render_regression_issue_payload(
        candidate,
        authorization.historical_owner.number,
        &authorization.recurrence_proof_digest,
        &public_context,
    )
    .map_err(|error| {
        pre_submit_commit_error(
            BlockedReason::Privacy,
            "RECAPTURE_SAFE_TYPED_EVIDENCE",
            privacy_as_owner_spec_error(error),
        )
    })?;
    if rendered != authorization.payload {
        return Err(pre_submit_commit_error(
            BlockedReason::Privacy,
            "RECAPTURE_SAFE_TYPED_EVIDENCE",
            owner_projection_error("authorized regression payload changed before owner creation"),
        ));
    }
    validate_source_occurrence_snapshot(&repo_root, candidate, &candidate.distinct_occurrences)
        .map_err(|error| {
            pre_submit_commit_error(BlockedReason::Store, "RELOAD_CANDIDATE_STORE", error)
        })?;

    let repository = RepositoryIdentity::gwt_upstream();
    let input = CreateRepositoryIssue {
        title: authorization.payload.title.clone(),
        body: authorization.payload.body.clone(),
        labels: Vec::new(),
    };
    let readback = env
        .improvement_owner_client(deadline)
        .map_err(pre_submit_commit_error_from_api)?
        .create_owner_issue(&repository, &input, deadline)
        .map_err(commit_error_from_mutation)?;
    Ok(CreatedOwnerMutation {
        readback,
        input,
        fingerprint,
    })
}

fn validate_regression_occurrence_keys(
    candidate: &ImprovementCandidate,
    occurrence_keys: &[String],
) -> Result<(), OwnerResolutionCommitError> {
    if occurrence_keys.is_empty()
        || occurrence_keys.windows(2).any(|pair| pair[0] >= pair[1])
        || occurrence_keys.iter().any(|key| {
            candidate
                .distinct_occurrences
                .iter()
                .find(|occurrence| occurrence.opaque_key == *key)
                .is_none_or(|occurrence| {
                    occurrence.recurrence.is_none() || !occurrence.qualifies_unattended
                })
        })
    {
        return Err(pre_submit_commit_error(
            BlockedReason::Ambiguity,
            "VERIFY_HISTORICAL_RECURRENCE",
            owner_projection_error(
                "regression authorization requires sorted recurrence occurrence keys",
            ),
        ));
    }
    Ok(())
}

fn commit_created_owner_readback<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    readback: &RepositoryIssue,
    input: &CreateRepositoryIssue,
    fingerprint: &str,
    deadline: &ResolutionDeadline,
) -> Result<(), OwnerResolutionCommitError> {
    let occurrence_keys = candidate
        .distinct_occurrences
        .iter()
        .map(|occurrence| occurrence.opaque_key.clone())
        .collect::<Vec<_>>();
    commit_created_owner_readback_for_occurrences(
        env,
        candidate,
        readback,
        input,
        fingerprint,
        CreatedOwnerOccurrenceCommit {
            occurrence_keys: &occurrence_keys,
            attempt_token: None,
        },
        deadline,
    )
}

struct CreatedOwnerOccurrenceCommit<'a> {
    occurrence_keys: &'a [String],
    attempt_token: Option<&'a ResolutionAttemptToken>,
}

fn commit_created_owner_readback_for_occurrences<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    readback: &RepositoryIssue,
    input: &CreateRepositoryIssue,
    fingerprint: &str,
    occurrence_commit: CreatedOwnerOccurrenceCommit<'_>,
    deadline: &ResolutionDeadline,
) -> Result<(), OwnerResolutionCommitError> {
    let CreatedOwnerOccurrenceCommit {
        occurrence_keys,
        attempt_token,
    } = occurrence_commit;
    let repo_root = env.repo_path().to_path_buf();
    ensure_candidate_attempt_current(&repo_root, candidate).map_err(|error| {
        remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error)
    })?;
    let public_context = PublicMutationContext::for_repo_with_deadline(&repo_root, deadline);
    validate_new_owner_readback(readback, input, fingerprint).map_err(|error| {
        remote_commit_error(BlockedReason::Readback, "REFRESH_OWNER_CORPUS", error)
    })?;
    let mut verified_owner = durable_owner_from_readback(readback, fingerprint);
    let reconciliation_cleanup_verified =
        verify_reconciliation_cleanup(env, candidate, verified_owner.number, deadline).map_err(
            |error| {
                remote_commit_error(
                    BlockedReason::Reconciliation,
                    "RECONCILE_DUPLICATE_OWNERS",
                    error,
                )
            },
        )?;
    validate_projection_owner_for_candidate(candidate, &verified_owner, &public_context).map_err(
        |error| remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error),
    )?;
    let occurrence_key_set = occurrence_keys
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if occurrence_key_set.is_empty() || occurrence_key_set.len() != occurrence_keys.len() {
        return Err(remote_commit_error(
            BlockedReason::LocalCommit,
            "REPAIR_OWNER_PROJECTION",
            owner_projection_error("created owner projection requires unique occurrence keys"),
        ));
    }
    let occurrences = occurrence_keys
        .iter()
        .map(|key| {
            candidate
                .distinct_occurrences
                .iter()
                .find(|occurrence| occurrence.opaque_key == *key)
                .cloned()
                .ok_or_else(|| {
                    remote_commit_error(
                        BlockedReason::LocalCommit,
                        "REPAIR_OWNER_PROJECTION",
                        owner_projection_error(
                            "created owner projection occurrence key is absent from the candidate",
                        ),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut projection_commits = occurrences
        .iter()
        .map(|occurrence| {
            prepare_owner_projection_commit_with_context(
                &repo_root,
                &ReadbackVerifiedOwnerBinding {
                    candidate_id: candidate.id.clone(),
                    owner: verified_owner.clone(),
                    occurrence_key: occurrence.opaque_key.clone(),
                    resolution_status: CandidateState::Created,
                    last_seen: occurrence.captured_at.clone(),
                },
                &public_context,
            )
            .map_err(|error| {
                remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let source_store = super::improvement_store::load_and_repair(&repo_root).map_err(|error| {
        remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error)
    })?;
    let source_scope_nonce = source_store.source_scope_nonce.as_deref().ok_or_else(|| {
        remote_commit_error(
            BlockedReason::LocalCommit,
            "REPAIR_OWNER_PROJECTION",
            owner_projection_error("source scope nonce is missing"),
        )
    })?;
    let source_reference = super::improvement_store::owner_projection_source_reference_digest(
        source_scope_nonce,
        &candidate.id,
        fingerprint,
    );
    let projected_occurrence_keys = super::improvement_store::load_owner_projection()
        .map_err(|error| {
            remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error)
        })?
        .owners
        .into_iter()
        .filter(|record| record.fingerprint == fingerprint)
        .flat_map(|record| record.source_references)
        .filter(|source| source.digest == source_reference)
        .flat_map(|source| source.occurrence_keys)
        .collect::<BTreeSet<_>>();
    let additional_occurrences = candidate
        .distinct_occurrences
        .iter()
        .filter(|occurrence| {
            !occurrence_key_set.contains(occurrence.opaque_key.as_str())
                && !projected_occurrence_keys.contains(&occurrence.opaque_key)
        })
        .cloned()
        .collect::<Vec<_>>();
    if !additional_occurrences.is_empty() {
        let token = attempt_token.ok_or_else(|| {
            remote_commit_error(
                BlockedReason::LocalCommit,
                "REPAIR_OWNER_PROJECTION",
                owner_projection_error(
                    "additional regression occurrences require an active resolution attempt",
                ),
            )
        })?;
        let create_intent = candidate
            .pending_create_resolution
            .clone()
            .or_else(|| {
                candidate
                    .attempt
                    .as_ref()
                    .filter(|attempt| attempt.remote_phase == AttemptRemotePhase::Submitted)
                    .map(|attempt| attempt.intent.clone())
            })
            .filter(|intent| {
                matches!(
                    intent,
                    ResolutionAttemptIntent::CreateRegressionIssue { .. }
                )
            })
            .ok_or_else(|| {
                remote_commit_error(
                    BlockedReason::LocalCommit,
                    "REPAIR_OWNER_PROJECTION",
                    owner_projection_error(
                        "additional regression occurrences require a submitted create journal",
                    ),
                )
            })?;
        let create_step_is_current = candidate.attempt.as_ref().is_some_and(|attempt| {
            attempt.remote_phase == AttemptRemotePhase::Submitted && attempt.intent == create_intent
        });
        if create_step_is_current {
            complete_resolution_attempt_step(&repo_root, candidate, token, &create_intent)?;
        }
        validate_source_occurrence_snapshot(&repo_root, candidate, &candidate.distinct_occurrences)
            .map_err(|error| {
                remote_commit_error(BlockedReason::Store, "RELOAD_CANDIDATE_STORE", error)
            })?;

        let payloads = additional_occurrences
            .iter()
            .map(|occurrence| {
                render_occurrence_comment_payload(
                    candidate,
                    &occurrence.opaque_key,
                    &public_context,
                )
                .map_err(|error| {
                    remote_commit_error(
                        BlockedReason::Privacy,
                        "RECAPTURE_SAFE_TYPED_EVIDENCE",
                        privacy_as_owner_spec_error(error),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let repository = RepositoryIdentity::gwt_upstream();
        let client = env
            .improvement_owner_client(deadline)
            .map_err(|error| active_api_commit_error(error, true))?;
        let mut comments = client
            .list_comments(&repository, readback.number, deadline)
            .map_err(|error| active_api_commit_error(error, true))?
            .items()
            .to_vec();
        for (occurrence, payload) in additional_occurrences.iter().zip(&payloads) {
            if comments.iter().any(|comment| {
                comment_matches_occurrence(comment, &occurrence.opaque_key, fingerprint)
            }) {
                continue;
            }
            let intent = ResolutionAttemptIntent::OccurrenceComments {
                owner_number: readback.number.0,
                occurrence_keys: vec![occurrence.opaque_key.clone()],
                public_payload_digest: public_payload_digest(
                    "gwt.improvement.owner-comment-intent.v1",
                    &[&occurrence.opaque_key, &payload.body],
                ),
            };
            mark_resolution_attempt_submitted(&repo_root, candidate, token, intent.clone())?;
            ensure_candidate_attempt_current(&repo_root, candidate).map_err(|error| {
                remote_commit_error(BlockedReason::Store, "RELOAD_CANDIDATE_STORE", error)
            })?;
            let created = client
                .create_owner_comment(&repository, readback.number, &payload.body, deadline)
                .map_err(|error| active_mutation_commit_error(error, true))?;
            if created.body != payload.body {
                return Err(remote_commit_error(
                    BlockedReason::Readback,
                    "REFRESH_OWNER_CORPUS",
                    owner_projection_error(
                        "regression occurrence comment readback did not match its payload",
                    ),
                ));
            }
            comments = client
                .list_comments(&repository, readback.number, deadline)
                .map_err(|error| active_api_commit_error(error, true))?
                .items()
                .to_vec();
            if !comments.iter().any(|comment| {
                comment_matches_occurrence(comment, &occurrence.opaque_key, fingerprint)
            }) {
                return Err(remote_commit_error(
                    BlockedReason::Readback,
                    "REFRESH_OWNER_CORPUS",
                    owner_projection_error(
                        "regression occurrence comment is absent from authoritative readback",
                    ),
                ));
            }
            complete_occurrence_comment_step(&repo_root, candidate, token, &intent)?;
        }

        let final_comments = client
            .list_comments(&repository, readback.number, deadline)
            .map_err(|error| active_api_commit_error(error, true))?;
        let final_readback = client
            .fetch_issue(&repository, readback.number, deadline)
            .map_err(|error| active_api_commit_error(error, true))?;
        validate_new_owner_readback(&final_readback, input, fingerprint).map_err(|error| {
            remote_commit_error(BlockedReason::Readback, "REFRESH_OWNER_CORPUS", error)
        })?;
        verified_owner = durable_owner_from_readback(&final_readback, fingerprint);
        validate_projection_owner_for_candidate(candidate, &verified_owner, &public_context)
            .map_err(|error| {
                remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error)
            })?;
        let projected_owner = owner_projection_owner(&verified_owner);
        for commit in &mut projection_commits {
            commit.owner = projected_owner.clone();
        }
        for occurrence in &additional_occurrences {
            let mut comment_ids = final_comments
                .items()
                .iter()
                .filter(|comment| {
                    comment_matches_occurrence(comment, &occurrence.opaque_key, fingerprint)
                })
                .map(|comment| comment.id.0)
                .collect::<Vec<_>>();
            comment_ids.sort_unstable();
            comment_ids.dedup();
            if comment_ids.is_empty() {
                return Err(remote_commit_error(
                    BlockedReason::Readback,
                    "REFRESH_OWNER_CORPUS",
                    owner_projection_error(
                        "regression occurrence comment is absent from final readback",
                    ),
                ));
            }
            let mut commit = prepare_owner_projection_commit_with_context(
                &repo_root,
                &ReadbackVerifiedOwnerBinding {
                    candidate_id: candidate.id.clone(),
                    owner: verified_owner.clone(),
                    occurrence_key: occurrence.opaque_key.clone(),
                    resolution_status: CandidateState::Linked,
                    last_seen: occurrence.captured_at.clone(),
                },
                &public_context,
            )
            .map_err(|error| {
                remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error)
            })?;
            commit.occurrence.comment_audit = super::improvement_store::StoredCommentAudit {
                completeness: super::improvement_store::StoredCommentAuditCompleteness::Complete,
                physical_comments: comment_ids
                    .into_iter()
                    .map(|comment_id| super::improvement_store::StoredCommentRef {
                        owner_number: verified_owner.number,
                        comment_id,
                    })
                    .collect(),
            };
            projection_commits.push(commit);
        }
    }

    let repaired = commit_owner_projection_and_source_success(
        &repo_root,
        candidate,
        projection_commits,
        verified_owner.number,
        reconciliation_cleanup_verified,
    )
    .map_err(|error| {
        remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error)
    })?;
    if repaired.state != CandidateState::Created
        || repaired
            .owner
            .as_ref()
            .is_none_or(|owner| owner.number != verified_owner.number)
    {
        return Err(remote_commit_error(
            BlockedReason::LocalCommit,
            "REPAIR_OWNER_PROJECTION",
            owner_projection_error(
                "source candidate projection coverage did not reach created state",
            ),
        ));
    }
    *candidate = repaired;
    deliver_pending_owner_status(env, candidate).map_err(OwnerResolutionCommitError::DurableStatus)
}

fn try_adopt_created_owner_resolution<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    owner: &OwnerCandidate,
    recovery_intent: &ResolutionAttemptIntent,
    deadline: &ResolutionDeadline,
) -> Result<bool, OwnerResolutionCommitError> {
    let ResolutionAttemptIntent::CreateIssue {
        fingerprint,
        public_payload_digest: expected_digest,
        created_owner_number,
    } = recovery_intent
    else {
        return Ok(false);
    };
    if candidate.fingerprint.as_deref() != Some(fingerprint.as_str())
        || !owner.active
        || !owner.selectable
        || owner.kind != OwnerKind::Issue
        || owner.match_basis != OwnerMatchBasis::Fingerprint
        || created_owner_number.is_some_and(|number| number != owner.number)
    {
        return Ok(false);
    }
    let repository = RepositoryIdentity::gwt_upstream();
    let readback = env
        .improvement_owner_client(deadline)
        .map_err(|error| {
            remote_commit_error(
                BlockedReason::Readback,
                "REFRESH_OWNER_CORPUS",
                error.into(),
            )
        })?
        .fetch_issue(&repository, IssueNumber(owner.number), deadline)
        .map_err(|error| {
            remote_commit_error(
                BlockedReason::Readback,
                "REFRESH_OWNER_CORPUS",
                error.into(),
            )
        })?;
    if !submitted_owner_readback_matches(&readback, fingerprint, expected_digest) {
        return Ok(false);
    }
    let input = CreateRepositoryIssue {
        title: readback.title.clone(),
        body: readback.body.clone(),
        labels: readback.labels.clone(),
    };
    commit_created_owner_readback(env, candidate, &readback, &input, fingerprint, deadline)?;
    Ok(true)
}

fn try_adopt_created_regression_owner_resolution<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    owner: &OwnerCandidate,
    recovery_intent: &ResolutionAttemptIntent,
    deadline: &ResolutionDeadline,
) -> Result<bool, OwnerResolutionCommitError> {
    let ResolutionAttemptIntent::CreateRegressionIssue {
        fingerprint,
        historical_owner_number,
        recurrence_occurrence_keys,
        recurrence_proof_digest,
        public_payload_digest: expected_digest,
        created_owner_number,
    } = recovery_intent
    else {
        return Ok(false);
    };
    if candidate.fingerprint.as_deref() != Some(fingerprint.as_str())
        || candidate
            .owner
            .as_ref()
            .is_none_or(|historical| historical.number != *historical_owner_number)
        || !owner.active
        || !owner.selectable
        || owner.kind != OwnerKind::Issue
        || owner.match_basis != OwnerMatchBasis::Fingerprint
        || created_owner_number.is_some_and(|number| number != owner.number)
    {
        return Ok(false);
    }
    if validate_regression_occurrence_keys(candidate, recurrence_occurrence_keys).is_err() {
        return Err(remote_commit_error(
            BlockedReason::Ambiguity,
            "VERIFY_HISTORICAL_RECURRENCE",
            owner_projection_error("submitted regression occurrence keys are invalid"),
        ));
    }
    let repository = RepositoryIdentity::gwt_upstream();
    let readback = env
        .improvement_owner_client(deadline)
        .map_err(|error| {
            remote_commit_error(
                BlockedReason::Readback,
                "REFRESH_OWNER_CORPUS",
                error.into(),
            )
        })?
        .fetch_issue(&repository, IssueNumber(owner.number), deadline)
        .map_err(|error| {
            remote_commit_error(
                BlockedReason::Readback,
                "REFRESH_OWNER_CORPUS",
                error.into(),
            )
        })?;
    if !submitted_regression_readback_matches(
        &readback,
        fingerprint,
        *historical_owner_number,
        recurrence_proof_digest,
        expected_digest,
    ) {
        return Ok(false);
    }
    let input = CreateRepositoryIssue {
        title: readback.title.clone(),
        body: readback.body.clone(),
        labels: readback.labels.clone(),
    };
    commit_created_owner_readback_for_occurrences(
        env,
        candidate,
        &readback,
        &input,
        fingerprint,
        CreatedOwnerOccurrenceCommit {
            occurrence_keys: recurrence_occurrence_keys,
            attempt_token: Some(token),
        },
        deadline,
    )?;
    Ok(true)
}

fn submitted_regression_readback_matches(
    readback: &RepositoryIssue,
    fingerprint: &str,
    historical_owner_number: u64,
    recurrence_proof_digest: &str,
    expected_payload_digest: &str,
) -> bool {
    let expected_marker = format!(
        "<!-- gwt:improvement-regression:v1 historical:{historical_owner_number} proof:{recurrence_proof_digest} -->"
    );
    let fingerprint_markers = exact_fingerprint_markers(&readback.body);
    readback.repository == RepositoryIdentity::gwt_upstream()
        && readback.number.0 != 0
        && readback.state == IssueState::Open
        && readback.kind == RepositoryIssueKind::Plain
        && readback.labels.is_empty()
        && fingerprint_markers.len() == 1
        && fingerprint_markers[0] == fingerprint
        && readback
            .body
            .lines()
            .filter(|line| *line == expected_marker)
            .count()
            == 1
        && public_payload_digest(
            "gwt.improvement.regression-create-intent.v1",
            &[&readback.title, &readback.body],
        ) == expected_payload_digest
}

fn submitted_owner_readback_matches(
    readback: &RepositoryIssue,
    fingerprint: &str,
    expected_payload_digest: &str,
) -> bool {
    let fingerprint_markers = exact_fingerprint_markers(&readback.body);
    readback.repository == RepositoryIdentity::gwt_upstream()
        && readback.number.0 != 0
        && readback.state == IssueState::Open
        && readback.kind == RepositoryIssueKind::Plain
        && readback.labels.is_empty()
        && fingerprint_markers.len() == 1
        && fingerprint_markers[0] == fingerprint
        && public_payload_digest(
            "gwt.improvement.owner-create-intent.v1",
            &[&readback.title, &readback.body],
        ) == expected_payload_digest
}

fn settle_submitted_create_before_reconciliation<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    root_intent: &ResolutionAttemptIntent,
    owners: &[OwnerCandidate],
    deadline: &ResolutionDeadline,
) -> Result<ResolutionAttemptIntent, OwnerResolutionCommitError> {
    let Some(attempt) = candidate.attempt.as_ref() else {
        return Ok(root_intent.clone());
    };
    if attempt.remote_phase != AttemptRemotePhase::Submitted
        || &attempt.intent != root_intent
        || !matches!(
            root_intent,
            ResolutionAttemptIntent::CreateIssue { .. }
                | ResolutionAttemptIntent::CreateRegressionIssue { .. }
        )
    {
        return Ok(root_intent.clone());
    }

    let repository = RepositoryIdentity::gwt_upstream();
    let matching_owner_numbers = {
        let client = env.improvement_owner_client(deadline).map_err(|error| {
            remote_commit_error(
                BlockedReason::Readback,
                "REFRESH_OWNER_CORPUS",
                error.into(),
            )
        })?;
        let mut matching_owner_numbers = Vec::new();
        for owner in owners {
            let readback = client
                .fetch_issue(&repository, IssueNumber(owner.number), deadline)
                .map_err(|error| {
                    remote_commit_error(
                        BlockedReason::Readback,
                        "REFRESH_OWNER_CORPUS",
                        error.into(),
                    )
                })?;
            let matches = match root_intent {
                ResolutionAttemptIntent::CreateIssue {
                    fingerprint,
                    public_payload_digest,
                    ..
                } => {
                    submitted_owner_readback_matches(&readback, fingerprint, public_payload_digest)
                }
                ResolutionAttemptIntent::CreateRegressionIssue {
                    fingerprint,
                    historical_owner_number,
                    recurrence_proof_digest,
                    public_payload_digest,
                    ..
                } => submitted_regression_readback_matches(
                    &readback,
                    fingerprint,
                    *historical_owner_number,
                    recurrence_proof_digest,
                    public_payload_digest,
                ),
                _ => false,
            };
            if matches {
                matching_owner_numbers.push(owner.number);
            }
        }
        matching_owner_numbers.sort_unstable();
        matching_owner_numbers.dedup();
        matching_owner_numbers
    };

    let unmatched_readback = || {
        remote_commit_error(
            BlockedReason::Readback,
            "REFRESH_OWNER_CORPUS",
            owner_projection_error("submitted create result did not match authoritative readback"),
        )
    };
    let settled_owner_number = match recorded_created_owner_number(root_intent) {
        Some(expected) if matching_owner_numbers.contains(&expected) => expected,
        None if matching_owner_numbers.len() == 1 => matching_owner_numbers[0],
        None if matching_owner_numbers.len() > 1 => {
            // The submitted create is settled, but an unnumbered response cannot
            // attribute any one of several byte-identical owners to this attempt.
            complete_resolution_attempt_step(env.repo_path(), candidate, token, root_intent)?;
            return Ok(root_intent.clone());
        }
        None => return Err(unmatched_readback()),
        _ => {
            return Err(unmatched_readback());
        }
    };

    let settled_intent =
        record_created_owner_readback(env.repo_path(), candidate, token, settled_owner_number)?;
    complete_resolution_attempt_step(env.repo_path(), candidate, token, &settled_intent)?;
    Ok(settled_intent)
}

fn validate_new_owner_readback(
    readback: &RepositoryIssue,
    input: &CreateRepositoryIssue,
    fingerprint: &str,
) -> Result<(), gwt_github::SpecOpsError> {
    if !new_owner_readback_matches(readback, input, fingerprint) {
        return Err(owner_projection_error(
            "new owner readback did not match the authorized plain Issue payload",
        ));
    }
    Ok(())
}

fn new_owner_readback_matches(
    readback: &RepositoryIssue,
    input: &CreateRepositoryIssue,
    fingerprint: &str,
) -> bool {
    readback.repository == RepositoryIdentity::gwt_upstream()
        && readback.number.0 != 0
        && readback.state == IssueState::Open
        && readback.kind == RepositoryIssueKind::Plain
        && readback.title == input.title
        && readback.body == input.body
        && readback.labels == input.labels
        && exact_fingerprint_markers(&readback.body)
            .iter()
            .any(|marker| marker == fingerprint)
}

fn pre_submit_commit_error_from_api(error: GitHubApiError) -> OwnerResolutionCommitError {
    let failure = owner_failure_from_api(&error);
    OwnerResolutionCommitError::PreSubmit {
        failure,
        source: error.into(),
        clear_create_root: false,
    }
}

fn privacy_as_owner_spec_error(error: PrivacyViolation) -> gwt_github::SpecOpsError {
    owner_projection_error(&format!(
        "owner public payload privacy validation failed: {error}"
    ))
}

fn commit_error_from_mutation(error: OwnerMutationError) -> OwnerResolutionCommitError {
    match error {
        OwnerMutationError::PreSubmit(error) => pre_submit_commit_error_from_api(error),
        OwnerMutationError::RemoteOutcomeUnknown(error) => {
            let failure = owner_failure_from_api(&error);
            OwnerResolutionCommitError::RemoteOutcomeUnknown {
                failure,
                source: error.into(),
            }
        }
    }
}

fn pre_submit_commit_error(
    reason: BlockedReason,
    remediation: &'static str,
    source: gwt_github::SpecOpsError,
) -> OwnerResolutionCommitError {
    OwnerResolutionCommitError::PreSubmit {
        failure: OwnerResolutionFailure {
            reason,
            failure_subcode: None,
            remediation,
        },
        source,
        clear_create_root: false,
    }
}

fn clear_create_root_on_pre_submit(
    mut error: OwnerResolutionCommitError,
) -> OwnerResolutionCommitError {
    if let OwnerResolutionCommitError::PreSubmit {
        clear_create_root, ..
    } = &mut error
    {
        *clear_create_root = true;
    }
    error
}

fn remote_commit_error(
    reason: BlockedReason,
    remediation: &'static str,
    source: gwt_github::SpecOpsError,
) -> OwnerResolutionCommitError {
    OwnerResolutionCommitError::RemoteOutcomeUnknown {
        failure: OwnerResolutionFailure {
            reason,
            failure_subcode: None,
            remediation,
        },
        source,
    }
}

fn pending_owner_status_generation(candidate: &ImprovementCandidate) -> Option<u64> {
    (candidate.owner_status_delivered_generation < candidate.owner_status_generation)
        .then_some(candidate.owner_status_generation)
}

fn acknowledge_owner_status(
    repo_root: &Path,
    candidate_id: &str,
    generation: u64,
    expected_owner_number: u64,
    expected_state: CandidateState,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    super::improvement_store::update(repo_root, |store| {
        let candidate = store
            .candidates
            .iter_mut()
            .find(|candidate| candidate.id == candidate_id)
            .ok_or_else(|| owner_projection_error("candidate not found"))?;
        if generation == 0 || generation > candidate.owner_status_generation {
            return Err(owner_projection_error(
                "owner status acknowledgement generation is invalid",
            ));
        }
        if candidate.state != expected_state
            || candidate
                .owner
                .as_ref()
                .is_none_or(|owner| owner.number != expected_owner_number)
        {
            return Err(owner_projection_error(
                "owner changed before status acknowledgement",
            ));
        }
        candidate.owner_status_delivered_generation =
            candidate.owner_status_delivered_generation.max(generation);
        super::improvement::validate_candidate_lifecycle(candidate)?;
        Ok(candidate.clone())
    })
}

pub(super) fn deliver_pending_owner_status<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
) -> Result<(), gwt_github::SpecOpsError> {
    let Some(generation) = pending_owner_status_generation(candidate) else {
        return Ok(());
    };
    let owner_number = candidate
        .owner
        .as_ref()
        .map(|owner| owner.number)
        .ok_or_else(|| owner_projection_error("pending owner status has no durable owner"))?;
    let state = candidate.state;
    match state {
        CandidateState::Linked => post_active_owner_linked_status(env, candidate, owner_number)?,
        CandidateState::Created => post_new_owner_created_status(env, candidate, owner_number)?,
        _ => {
            return Err(owner_projection_error(
                "pending owner status requires a successful owner state",
            ))
        }
    }
    *candidate = acknowledge_owner_status(
        env.repo_path(),
        &candidate.id,
        generation,
        owner_number,
        state,
    )?;
    Ok(())
}

pub(super) fn retry_pending_owner_status_with_deadline<E: CliEnv>(
    env: &mut E,
    candidate_id: &str,
    deadline: &ResolutionDeadline,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    let _operation_deadline =
        gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline.expires_at());
    deadline.remaining("pending owner status entry")?;
    let repo_root = env.repo_path().to_path_buf();
    repair_source_success_snapshots(&repo_root)?;
    let mut candidate = super::improvement_store::load_and_repair(&repo_root)?
        .candidates
        .into_iter()
        .find(|candidate| candidate.id == candidate_id)
        .ok_or_else(|| owner_projection_error("candidate not found"))?;
    if !matches!(
        candidate.state,
        CandidateState::Linked | CandidateState::Created
    ) {
        return Err(owner_projection_error(
            "pending owner status requires a successful owner state",
        ));
    }
    deliver_pending_owner_status(env, &mut candidate)?;
    deadline.remaining("pending owner status completion")?;
    Ok(candidate)
}

fn post_new_owner_created_status<E: CliEnv>(
    env: &mut E,
    candidate: &ImprovementCandidate,
    owner_number: u64,
) -> Result<(), gwt_github::SpecOpsError> {
    post_improvement_board_status(
        env,
        format!(
            "Current state: Improvement Candidate {id} owner #{owner_number} was created in akiojin/gwt.\n\nReason: complete authoritative zero and a revision-pinned IMPLEMENTATION-GAP route authorized the typed Issue; readback and projection-first local commit succeeded.\n\nNext: Track the owner at https://github.com/akiojin/gwt/issues/{owner_number}.",
            id = candidate.id,
        ),
    )
}

fn merge_recovered_comment_evidence(
    outcome: &mut OwnerPreflightOutcome,
    evidence: Vec<(IssueNumber, RepositoryComment)>,
) {
    fn merge(comments: &mut Vec<RepositoryComment>, comment: RepositoryComment) {
        if !comments
            .iter()
            .any(|existing| existing.id == comment.id && existing.body == comment.body)
        {
            comments.push(comment);
            comments.sort_by_key(|comment| comment.id);
        }
    }

    for (owner_number, comment) in evidence {
        match outcome {
            OwnerPreflightOutcome::Active {
                owner, comments, ..
            } if owner.number == owner_number.0 => merge(comments, comment),
            OwnerPreflightOutcome::Historical {
                comments_by_owner, ..
            }
            | OwnerPreflightOutcome::DuplicateExactIssues {
                comments_by_owner, ..
            } => merge(comments_by_owner.entry(owner_number).or_default(), comment),
            _ => {}
        }
    }
}

pub(super) fn resolve_candidate_owner<E: CliEnv>(
    env: &mut E,
    candidate_id: &str,
    budget_profile: CaptureBudgetProfile,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    let deadline = budget_profile.resolution_deadline();
    resolve_candidate_owner_with_deadline(env, candidate_id, budget_profile, &deadline)
}

pub(super) fn resolve_candidate_owner_with_deadline<E: CliEnv>(
    env: &mut E,
    candidate_id: &str,
    budget_profile: CaptureBudgetProfile,
    deadline: &ResolutionDeadline,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    resolve_candidate_owner_with_operation_deadline(
        env,
        candidate_id,
        budget_profile,
        deadline,
        deadline.expires_at(),
    )
}

pub(super) fn resolve_candidate_owner_with_operation_deadline<E: CliEnv>(
    env: &mut E,
    candidate_id: &str,
    budget_profile: CaptureBudgetProfile,
    deadline: &ResolutionDeadline,
    operation_expires_at: Instant,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    let _operation_deadline =
        gwt_core::operation_deadline::ScopedOperationDeadline::enter(operation_expires_at);
    let unhandled_settlement = std::cell::OnceCell::new();
    let _resolution_operation_deadline =
        gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline.expires_at());
    deadline.remaining("owner resolution entry")?;
    let repo_root = env.repo_path().to_path_buf();
    repair_source_success_snapshots(&repo_root)?;
    let mut current = super::improvement_store::load_and_repair(&repo_root)?
        .candidates
        .into_iter()
        .find(|candidate| candidate.id == candidate_id)
        .ok_or_else(|| owner_projection_error("candidate not found"))?;
    if matches!(
        current.state,
        CandidateState::Linked | CandidateState::Created
    ) {
        deliver_pending_owner_status(env, &mut current)?;
    }
    let (
        mut candidate,
        attempt_token,
        recovering_remote_unknown,
        recovery_intent,
        reconciliation_only,
    ) = match begin_resolution_attempt(&repo_root, candidate_id, budget_profile)? {
        ResolutionAttemptStart::Acquired {
            candidate,
            token,
            recovering_remote_unknown,
            recovery_intent,
            reconciliation_only,
        } => (
            candidate,
            token,
            recovering_remote_unknown,
            recovery_intent,
            reconciliation_only,
        ),
    };
    assert!(
        unhandled_settlement
            .set(UnhandledResolutionSettlement {
                repo_root: repo_root.clone(),
                token: attempt_token.clone(),
                resolution_deadline: *deadline,
            })
            .is_ok(),
        "owner resolution settlement guard must be installed exactly once"
    );

    let mut recovery_pending = recovering_remote_unknown;
    let mut recovered_comment_evidence = Vec::new();
    if recovery_pending
        && matches!(
            recovery_intent,
            ResolutionAttemptIntent::ReconciliationComment { .. }
                | ResolutionAttemptIntent::CloseDuplicate { .. }
        )
    {
        match adopt_recovered_reconciliation_step(
            env,
            &mut candidate,
            &attempt_token,
            &recovery_intent,
            deadline,
        ) {
            Ok(Some(evidence)) => recovered_comment_evidence.push(evidence),
            Ok(None) => {}
            Err(error) => {
                return settle_owner_commit_error(env, candidate, &attempt_token, error);
            }
        }
        recovery_pending = false;
    }
    if recovery_pending
        && matches!(
            recovery_intent,
            ResolutionAttemptIntent::OccurrenceComments { .. }
        )
    {
        match adopt_recovered_occurrence_comments_step(
            env,
            &mut candidate,
            &attempt_token,
            &recovery_intent,
            deadline,
        ) {
            Ok(Some(evidence)) => recovered_comment_evidence.push(evidence),
            Ok(None) => {}
            Err(error) => {
                return settle_owner_commit_error(env, candidate, &attempt_token, error);
            }
        }
        recovery_pending = false;
    }
    let mut outcome = owner_resolution_preflight_deferred(
        env,
        &mut candidate,
        &ContractRoutingRegistry::current(),
        &NoSemanticOwnerAdvisor,
        deadline,
    )?;
    merge_recovered_comment_evidence(&mut outcome, recovered_comment_evidence);
    let mut create_resolution_intent = candidate.pending_create_resolution.clone().or_else(|| {
        (recovery_pending
            && matches!(
                recovery_intent,
                ResolutionAttemptIntent::CreateIssue { .. }
                    | ResolutionAttemptIntent::CreateRegressionIssue { .. }
            ))
        .then(|| recovery_intent.clone())
    });
    match outcome {
        OwnerPreflightOutcome::Active {
            owner, comments, ..
        } => {
            if let Some(created_owner_number) = create_resolution_intent
                .as_ref()
                .and_then(recorded_created_owner_number)
                .filter(|number| *number != owner.number)
            {
                if let Err(error) = arm_reconciliation_required(
                    &repo_root,
                    &mut candidate,
                    &attempt_token,
                    &[owner.number, created_owner_number],
                ) {
                    return settle_owner_commit_error(
                        env,
                        candidate,
                        &attempt_token,
                        pre_submit_commit_error(
                            BlockedReason::Reconciliation,
                            "RECONCILE_DUPLICATE_OWNERS",
                            error,
                        ),
                    );
                }
            }
            resolve_active_owner_outcome(
                env,
                candidate,
                &attempt_token,
                owner,
                comments,
                create_resolution_intent.as_ref(),
                deadline,
            )
        }
        OwnerPreflightOutcome::DuplicateExactIssues {
            owners,
            comments_by_owner,
            ..
        } => {
            if let Some(intent) = create_resolution_intent.clone() {
                match settle_submitted_create_before_reconciliation(
                    env,
                    &mut candidate,
                    &attempt_token,
                    &intent,
                    &owners,
                    deadline,
                ) {
                    Ok(settled_intent) => {
                        create_resolution_intent = Some(settled_intent);
                    }
                    Err(error) => {
                        return settle_owner_commit_error(env, candidate, &attempt_token, error);
                    }
                }
            }
            if let Some(created_owner_number) = create_resolution_intent
                .as_ref()
                .and_then(recorded_created_owner_number)
            {
                let mut known_owner_numbers =
                    owners.iter().map(|owner| owner.number).collect::<Vec<_>>();
                known_owner_numbers.push(created_owner_number);
                known_owner_numbers.sort_unstable();
                known_owner_numbers.dedup();
                if known_owner_numbers.len() > 1 {
                    if let Err(error) = arm_reconciliation_required(
                        &repo_root,
                        &mut candidate,
                        &attempt_token,
                        &known_owner_numbers,
                    ) {
                        return settle_owner_commit_error(
                            env,
                            candidate,
                            &attempt_token,
                            pre_submit_commit_error(
                                BlockedReason::Reconciliation,
                                "RECONCILE_DUPLICATE_OWNERS",
                                error,
                            ),
                        );
                    }
                }
            }
            match reconcile_duplicate_owners(
                env,
                &mut candidate,
                &attempt_token,
                owners,
                comments_by_owner,
                deadline,
            ) {
                Ok((owner, comments)) => resolve_active_owner_outcome(
                    env,
                    candidate,
                    &attempt_token,
                    owner,
                    comments,
                    create_resolution_intent.as_ref(),
                    deadline,
                ),
                Err(error) => settle_owner_commit_error(env, candidate, &attempt_token, error),
            }
        }
        OwnerPreflightOutcome::RegressionCreateAuthorized { authorization } => {
            if candidate
                .owner
                .as_ref()
                .is_none_or(|owner| owner.number != authorization.historical_owner.number)
            {
                let failure = OwnerResolutionFailure {
                    reason: BlockedReason::Ambiguity,
                    failure_subcode: None,
                    remediation: "VERIFY_HISTORICAL_RECURRENCE",
                };
                block_owner_resolution(env, &mut candidate, failure, None, false)?;
                candidate.attempt = None;
                return persist_and_post_resolver_failure(env, candidate, &attempt_token, failure);
            }
            if recovery_pending {
                return preserve_remote_unknown_recovery(
                    env,
                    candidate,
                    &attempt_token,
                    &recovery_intent,
                    OwnerResolutionFailure {
                        reason: BlockedReason::Readback,
                        failure_subcode: None,
                        remediation: "REFRESH_OWNER_CORPUS",
                    },
                );
            }
            if reconciliation_only {
                let failure = OwnerResolutionFailure {
                    reason: BlockedReason::Reconciliation,
                    failure_subcode: None,
                    remediation: "RECONCILE_DUPLICATE_OWNERS",
                };
                block_owner_resolution(env, &mut candidate, failure, None, false)?;
                candidate.attempt = None;
                return persist_and_post_resolver_failure(env, candidate, &attempt_token, failure);
            }
            let adoption_intent = regression_owner_attempt_intent(&authorization);
            let refreshed = owner_resolution_preflight_deferred(
                env,
                &mut candidate,
                &ContractRoutingRegistry::current(),
                &NoSemanticOwnerAdvisor,
                deadline,
            )?;
            let authorization = match refreshed {
                OwnerPreflightOutcome::Active {
                    owner, comments, ..
                } => {
                    return resolve_active_owner_outcome(
                        env,
                        candidate,
                        &attempt_token,
                        owner,
                        comments,
                        Some(&adoption_intent),
                        deadline,
                    );
                }
                OwnerPreflightOutcome::DuplicateExactIssues {
                    owners,
                    comments_by_owner,
                    ..
                } => {
                    return match reconcile_duplicate_owners(
                        env,
                        &mut candidate,
                        &attempt_token,
                        owners,
                        comments_by_owner,
                        deadline,
                    ) {
                        Ok((owner, comments)) => resolve_active_owner_outcome(
                            env,
                            candidate,
                            &attempt_token,
                            owner,
                            comments,
                            Some(&adoption_intent),
                            deadline,
                        ),
                        Err(error) => {
                            settle_owner_commit_error(env, candidate, &attempt_token, error)
                        }
                    };
                }
                OwnerPreflightOutcome::RegressionCreateAuthorized { authorization } => {
                    authorization
                }
                outcome => {
                    return settle_non_owner_preflight(
                        env,
                        candidate,
                        &attempt_token,
                        outcome,
                        None,
                        false,
                    );
                }
            };
            let create_intent = regression_owner_attempt_intent(&authorization);
            if let Err(error) = mark_resolution_attempt_submitted(
                &repo_root,
                &mut candidate,
                &attempt_token,
                create_intent.clone(),
            ) {
                return settle_owner_commit_error(env, candidate, &attempt_token, error);
            }
            match create_regression_owner_with_postflight(
                env,
                &mut candidate,
                &attempt_token,
                &authorization,
                deadline,
            ) {
                Ok(NewOwnerPostflight::Committed) => Ok(candidate),
                Ok(NewOwnerPostflight::Active { owner, comments }) => {
                    let adoption_intent = candidate.pending_create_resolution.clone();
                    resolve_active_owner_outcome(
                        env,
                        candidate,
                        &attempt_token,
                        owner,
                        comments,
                        adoption_intent.as_ref(),
                        deadline,
                    )
                }
                Err(error) => settle_owner_commit_error(env, candidate, &attempt_token, error),
            }
        }
        OwnerPreflightOutcome::CreateAuthorized { authorization } => {
            if candidate.owner.is_some() || candidate.linked_issue.is_some() {
                let failure = OwnerResolutionFailure {
                    reason: BlockedReason::Readback,
                    failure_subcode: None,
                    remediation: "REFRESH_OWNER_CORPUS",
                };
                block_owner_resolution(env, &mut candidate, failure, None, false)?;
                candidate.attempt = None;
                return persist_and_post_resolver_failure(env, candidate, &attempt_token, failure);
            }
            if recovery_pending {
                return preserve_remote_unknown_recovery(
                    env,
                    candidate,
                    &attempt_token,
                    &recovery_intent,
                    OwnerResolutionFailure {
                        reason: BlockedReason::Readback,
                        failure_subcode: None,
                        remediation: "REFRESH_OWNER_CORPUS",
                    },
                );
            }
            if reconciliation_only {
                let failure = OwnerResolutionFailure {
                    reason: BlockedReason::Reconciliation,
                    failure_subcode: None,
                    remediation: "RECONCILE_DUPLICATE_OWNERS",
                };
                block_owner_resolution(env, &mut candidate, failure, None, false)?;
                candidate.attempt = None;
                return persist_and_post_resolver_failure(env, candidate, &attempt_token, failure);
            }
            let adoption_intent = new_owner_attempt_intent(&authorization);
            let refreshed = owner_resolution_preflight_deferred(
                env,
                &mut candidate,
                &ContractRoutingRegistry::current(),
                &NoSemanticOwnerAdvisor,
                deadline,
            )?;
            let authorization = match refreshed {
                OwnerPreflightOutcome::Active {
                    owner, comments, ..
                } => {
                    return resolve_active_owner_outcome(
                        env,
                        candidate,
                        &attempt_token,
                        owner,
                        comments,
                        Some(&adoption_intent),
                        deadline,
                    );
                }
                OwnerPreflightOutcome::DuplicateExactIssues {
                    owners,
                    comments_by_owner,
                    ..
                } => {
                    return match reconcile_duplicate_owners(
                        env,
                        &mut candidate,
                        &attempt_token,
                        owners,
                        comments_by_owner,
                        deadline,
                    ) {
                        Ok((owner, comments)) => resolve_active_owner_outcome(
                            env,
                            candidate,
                            &attempt_token,
                            owner,
                            comments,
                            Some(&adoption_intent),
                            deadline,
                        ),
                        Err(error) => {
                            settle_owner_commit_error(env, candidate, &attempt_token, error)
                        }
                    };
                }
                OwnerPreflightOutcome::CreateAuthorized { authorization } => authorization,
                outcome => {
                    return settle_non_owner_preflight(
                        env,
                        candidate,
                        &attempt_token,
                        outcome,
                        None,
                        false,
                    );
                }
            };
            let create_intent = new_owner_attempt_intent(&authorization);
            if let Err(error) = mark_resolution_attempt_submitted(
                &repo_root,
                &mut candidate,
                &attempt_token,
                create_intent.clone(),
            ) {
                return settle_owner_commit_error(env, candidate, &attempt_token, error);
            }
            match create_owner_with_postflight(
                env,
                &mut candidate,
                &attempt_token,
                &authorization,
                deadline,
            ) {
                Ok(NewOwnerPostflight::Committed) => Ok(candidate),
                Ok(NewOwnerPostflight::Active { owner, comments }) => {
                    let adoption_intent = candidate.pending_create_resolution.clone();
                    resolve_active_owner_outcome(
                        env,
                        candidate,
                        &attempt_token,
                        owner,
                        comments,
                        adoption_intent.as_ref(),
                        deadline,
                    )
                }
                Err(error) => settle_owner_commit_error(env, candidate, &attempt_token, error),
            }
        }
        outcome => settle_non_owner_preflight(
            env,
            candidate,
            &attempt_token,
            outcome,
            recovery_pending.then_some(&recovery_intent),
            reconciliation_only,
        ),
    }
}

pub(super) fn select_candidate_owner<E: CliEnv>(
    env: &mut E,
    candidate_id: &str,
    selected_owner_number: u64,
    resolver_revision: &str,
    budget_profile: CaptureBudgetProfile,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    if selected_owner_number == 0
        || resolver_revision.len() != 64
        || !resolver_revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(owner_projection_error(
            "manual owner selection requires a positive owner and canonical resolver revision",
        ));
    }
    let repo_root = env.repo_path().to_path_buf();
    repair_source_success_snapshots(&repo_root)?;
    let stored_candidate = super::improvement_store::load_and_repair(&repo_root)?
        .candidates
        .into_iter()
        .find(|candidate| candidate.id == candidate_id)
        .ok_or_else(|| owner_projection_error("candidate not found"))?;
    let stored_snapshot = stored_candidate
        .resolver_snapshot
        .as_ref()
        .filter(|snapshot| snapshot.resolver_revision == resolver_revision)
        .ok_or_else(|| owner_projection_error("manual owner selection revision is stale"))?;
    let stored_owner = stored_snapshot
        .owner_candidates
        .iter()
        .find(|owner| owner.number == selected_owner_number)
        .filter(|owner| {
            owner.active && owner.selectable && owner.match_basis != OwnerMatchBasis::Semantic
        })
        .ok_or_else(|| {
            owner_projection_error(
                "manual owner selection requires a selectable active authoritative owner",
            )
        })?;
    if stored_candidate.state != CandidateState::Blocked
        || stored_candidate.blocked_reason != Some(BlockedReason::Ambiguity)
    {
        return Err(owner_projection_error(
            "manual owner selection requires a blocked ambiguity candidate",
        ));
    }

    let deadline = budget_profile.resolution_deadline();
    let _operation_deadline =
        gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline.expires_at());
    let source_scope_nonce = env.improvement_source_scope_nonce()?;
    let public_context = PublicMutationContext::for_repo_with_deadline(&repo_root, &deadline);
    if let Some(failure) = candidate_preflight_failure(
        &stored_candidate,
        &repo_root,
        &source_scope_nonce,
        &public_context,
    ) {
        return Err(owner_projection_error(&format!(
            "manual owner selection failed the {} safety gate",
            blocked_reason_token(failure.reason)
        )));
    }

    let fresh_inspection = env
        .improvement_owner_client(&deadline)
        .map_err(gwt_github::SpecOpsError::from)
        .and_then(|client| {
            inspect_owner_corpus(
                client,
                &stored_candidate,
                &ContractRoutingRegistry::current(),
                &NoSemanticOwnerAdvisor,
                &deadline,
            )
            .map_err(|failure| {
                owner_projection_error(&format!(
                    "manual owner selection refresh failed: {}",
                    blocked_reason_token(failure.reason)
                ))
            })
        })?;
    let OwnerInspection::Ambiguous {
        owner_candidates,
        corpus_generation,
    } = fresh_inspection
    else {
        return Err(owner_projection_error(
            "manual owner selection fresh revision has no ambiguous active owner set",
        ));
    };
    let fresh_snapshot = ResolverSnapshot::new(corpus_generation, owner_candidates)?;
    if fresh_snapshot.resolver_revision != resolver_revision {
        return Err(owner_projection_error(
            "manual owner selection fresh resolver revision is stale",
        ));
    }
    let selected_owner = fresh_snapshot
        .owner_candidates
        .iter()
        .find(|owner| owner.number == selected_owner_number)
        .filter(|owner| {
            owner.active
                && owner.selectable
                && owner.match_basis != OwnerMatchBasis::Semantic
                && *owner == stored_owner
        })
        .cloned()
        .ok_or_else(|| {
            owner_projection_error(
                "manual owner selection fresh revision changed the selected active owner",
            )
        })?;
    let selected_comments = env
        .improvement_owner_client(&deadline)
        .map_err(gwt_github::SpecOpsError::from)?
        .list_comments(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(selected_owner_number),
            &deadline,
        )
        .map_err(gwt_github::SpecOpsError::from)?;
    if selected_comments.generation().as_str().is_empty() {
        return Err(owner_projection_error(
            "manual owner selection comment corpus is incomplete",
        ));
    }

    let (mut candidate, token) =
        match begin_resolution_attempt(&repo_root, candidate_id, budget_profile)? {
            ResolutionAttemptStart::Acquired {
                candidate, token, ..
            } => (candidate, token),
        };
    if candidate.fingerprint != stored_candidate.fingerprint
        || candidate.distinct_occurrences != stored_candidate.distinct_occurrences
        || candidate
            .resolver_snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.resolver_revision != resolver_revision)
    {
        let error = pre_submit_commit_error(
            BlockedReason::Store,
            "RELOAD_CANDIDATE_STORE",
            owner_projection_error("candidate changed during manual owner selection"),
        );
        settle_owner_commit_error(env, candidate, &token, error)?;
        return Err(owner_projection_error(
            "candidate changed during manual owner selection",
        ));
    }
    match commit_active_owner_resolution_with_audit(
        env,
        &mut candidate,
        &selected_owner,
        selected_comments.items(),
        &deadline,
        Some(&token),
        Some(ManualOwnerSelectionAudit {
            token: &token,
            snapshot: &fresh_snapshot,
        }),
    ) {
        Ok(()) => Ok(candidate),
        Err(error) => settle_owner_commit_error(env, candidate, &token, error),
    }
}

fn append_manual_owner_selection_audit(
    repo_root: &Path,
    candidate: &ImprovementCandidate,
    token: &ResolutionAttemptToken,
    owner_number: u64,
    snapshot: &ResolverSnapshot,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    let entry = ImprovementAuditEntry::ManualOwnerSelection {
        owner_number,
        resolver_revision: snapshot.resolver_revision.clone(),
        corpus_generation: snapshot.corpus_generation.clone(),
        recorded_at: Utc::now().to_rfc3339(),
    };
    super::improvement_store::update(repo_root, |store| {
        let stored = store
            .candidates
            .iter_mut()
            .find(|stored| stored.id == candidate.id)
            .ok_or_else(|| owner_projection_error("candidate not found"))?;
        if stored.fingerprint != candidate.fingerprint
            || stored.distinct_occurrences != candidate.distinct_occurrences
            || stored.pending_create_resolution != candidate.pending_create_resolution
            || stored.state != CandidateState::OwnerResolving
            || stored.state != candidate.state
            || stored.dismissed_reason != candidate.dismissed_reason
            || stored.audit != candidate.audit
            || stored
                .attempt
                .as_ref()
                .is_none_or(|attempt| attempt.attempt_id != token.attempt_id)
            || stored
                .resolver_snapshot
                .as_ref()
                .is_none_or(|stored_snapshot| {
                    stored_snapshot.resolver_revision != snapshot.resolver_revision
                })
        {
            return Err(owner_projection_error(
                "candidate changed before manual owner selection audit",
            ));
        }
        stored.audit.push(entry);
        stored.updated_at = Utc::now().to_rfc3339();
        super::improvement::validate_candidate_lifecycle(stored)?;
        Ok(stored.clone())
    })
}

fn resolve_active_owner_outcome<E: CliEnv>(
    env: &mut E,
    mut candidate: ImprovementCandidate,
    token: &ResolutionAttemptToken,
    owner: OwnerCandidate,
    comments: Vec<RepositoryComment>,
    mut adoption_intent: Option<&ResolutionAttemptIntent>,
    deadline: &ResolutionDeadline,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    let repo_root = env.repo_path().to_path_buf();
    if let Err(error) = renew_resolution_attempt(&repo_root, &mut candidate, token) {
        return settle_owner_commit_error(env, candidate, token, error);
    }
    if adoption_intent.is_some_and(|intent| {
        recorded_created_owner_number(intent).is_none()
            && candidate.reconciliation_required
            && candidate.reconciliation_owner_numbers.len() > 1
    }) {
        adoption_intent = None;
    }
    if let Some(
        intent @ ResolutionAttemptIntent::CreateRegressionIssue {
            created_owner_number,
            ..
        },
    ) = adoption_intent
    {
        let adoption_is_submitted_recovery = candidate.attempt.as_ref().is_some_and(|attempt| {
            attempt.remote_phase == AttemptRemotePhase::Submitted && &attempt.intent == intent
        });
        let adoption_is_pending_root = candidate.pending_create_resolution.as_ref() == Some(intent);
        if adoption_is_submitted_recovery || adoption_is_pending_root {
            match try_adopt_created_regression_owner_resolution(
                env,
                &mut candidate,
                token,
                &owner,
                intent,
                deadline,
            ) {
                Ok(true) => return Ok(candidate),
                Ok(false)
                    if created_owner_number.is_some_and(|created_owner_number| {
                        created_owner_number != owner.number
                            && candidate.reconciliation_required
                            && candidate
                                .reconciliation_owner_numbers
                                .contains(&created_owner_number)
                            && candidate
                                .reconciliation_owner_numbers
                                .contains(&owner.number)
                    }) => {}
                Ok(false) => {
                    let error = remote_commit_error(
                        BlockedReason::Readback,
                        "REFRESH_OWNER_CORPUS",
                        owner_projection_error(
                            "submitted regression owner did not match authoritative readback",
                        ),
                    );
                    return settle_owner_commit_error(env, candidate, token, error);
                }
                Err(error) => return settle_owner_commit_error(env, candidate, token, error),
            }
        }
    }
    if let Some(
        intent @ ResolutionAttemptIntent::CreateIssue {
            created_owner_number,
            ..
        },
    ) = adoption_intent
    {
        let adoption_is_submitted_recovery = candidate.attempt.as_ref().is_some_and(|attempt| {
            attempt.remote_phase == AttemptRemotePhase::Submitted && &attempt.intent == intent
        });
        let adoption_is_pending_root = candidate.pending_create_resolution.as_ref() == Some(intent);
        if adoption_is_submitted_recovery || adoption_is_pending_root {
            match try_adopt_created_owner_resolution(env, &mut candidate, &owner, intent, deadline)
            {
                Ok(true) => return Ok(candidate),
                Ok(false) if created_owner_number.is_none() => {
                    let error = remote_commit_error(
                        BlockedReason::Readback,
                        "REFRESH_OWNER_CORPUS",
                        owner_projection_error(
                            "submitted plain owner did not match authoritative readback",
                        ),
                    );
                    return settle_owner_commit_error(env, candidate, token, error);
                }
                Ok(false) => {}
                Err(error) => return settle_owner_commit_error(env, candidate, token, error),
            }
        }
    }
    let (owner, comments) = match reconcile_visible_owner_with_durable_owner(
        env,
        &mut candidate,
        token,
        owner,
        comments,
        deadline,
    ) {
        Ok(resolved) => resolved,
        Err(error) => return settle_owner_commit_error(env, candidate, token, error),
    };
    match commit_active_owner_resolution_for_attempt(
        env,
        &mut candidate,
        token,
        &owner,
        &comments,
        deadline,
    ) {
        Ok(()) => Ok(candidate),
        Err(error) => settle_owner_commit_error(env, candidate, token, error),
    }
}

fn reconcile_visible_owner_with_durable_owner<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    visible_owner: OwnerCandidate,
    visible_comments: Vec<RepositoryComment>,
    deadline: &ResolutionDeadline,
) -> Result<(OwnerCandidate, Vec<RepositoryComment>), OwnerResolutionCommitError> {
    let durable_owner_number = candidate
        .owner
        .as_ref()
        .map(|owner| owner.number)
        .or_else(|| candidate.linked_issue.as_ref().map(|owner| owner.number));
    let Some(durable_owner_number) = durable_owner_number else {
        return Ok((visible_owner, visible_comments));
    };
    if durable_owner_number == visible_owner.number {
        return Ok((visible_owner, visible_comments));
    }
    if candidate.reconciliation_required
        && candidate
            .reconciliation_owner_numbers
            .contains(&visible_owner.number)
        && candidate
            .reconciliation_owner_numbers
            .contains(&durable_owner_number)
    {
        return Ok((visible_owner, visible_comments));
    }
    let fingerprint = candidate
        .fingerprint
        .as_deref()
        .ok_or_else(|| reconciliation_pre_submit_error("candidate fingerprint is missing"))?;
    let repository = RepositoryIdentity::gwt_upstream();
    let durable_readback = env
        .improvement_owner_client(deadline)
        .map_err(reconciliation_pre_submit_error_from_api)?
        .fetch_issue(&repository, IssueNumber(durable_owner_number), deadline)
        .map_err(reconciliation_pre_submit_error_from_api)?;
    if durable_readback.repository == repository
        && durable_readback.number == IssueNumber(durable_owner_number)
        && durable_readback.state == IssueState::Closed
        && matches!(
            durable_readback.kind,
            RepositoryIssueKind::Plain | RepositoryIssueKind::Spec
        )
        && exact_fingerprint_markers(&durable_readback.body)
            .iter()
            .any(|marker| marker == fingerprint)
    {
        return Ok((visible_owner, visible_comments));
    }
    let durable_owner = owner_candidate(&durable_readback, OwnerMatchBasis::Fingerprint);
    validate_active_owner_readback(&durable_owner, fingerprint, &durable_readback)
        .map_err(|error| reconciliation_pre_submit_error(&error.to_string()))?;
    let durable_comments = env
        .improvement_owner_client(deadline)
        .map_err(reconciliation_pre_submit_error_from_api)?
        .list_comments(&repository, IssueNumber(durable_owner_number), deadline)
        .map_err(reconciliation_pre_submit_error_from_api)?
        .items()
        .to_vec();
    let mut comments_by_owner = BTreeMap::new();
    comments_by_owner.insert(IssueNumber(visible_owner.number), visible_comments);
    comments_by_owner.insert(IssueNumber(durable_owner_number), durable_comments);
    reconcile_duplicate_owners(
        env,
        candidate,
        token,
        vec![visible_owner, durable_owner],
        comments_by_owner,
        deadline,
    )
}

fn reconcile_duplicate_owners<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    mut owners: Vec<OwnerCandidate>,
    mut comments_by_owner: BTreeMap<IssueNumber, Vec<RepositoryComment>>,
    deadline: &ResolutionDeadline,
) -> Result<(OwnerCandidate, Vec<RepositoryComment>), OwnerResolutionCommitError> {
    owners.sort_by_key(|owner| owner.number);
    if owners.len() < 2
        || owners.iter().any(|owner| {
            !owner.active
                || !owner.selectable
                || owner.kind != OwnerKind::Issue
                || owner.match_basis != OwnerMatchBasis::Fingerprint
        })
    {
        return Err(reconciliation_pre_submit_error(
            "duplicate reconciliation requires multiple exact active plain Issues",
        ));
    }
    let reconciliation_owner_numbers = owners.iter().map(|owner| owner.number).collect::<Vec<_>>();
    arm_reconciliation_required(
        env.repo_path(),
        candidate,
        token,
        &reconciliation_owner_numbers,
    )
    .map_err(|error| {
        pre_submit_commit_error(
            BlockedReason::Reconciliation,
            "RECONCILE_DUPLICATE_OWNERS",
            error,
        )
    })?;
    let canonical_number = owners[0].number;
    let canonical = owners[0].clone();
    let fingerprint = candidate
        .fingerprint
        .as_deref()
        .ok_or_else(|| reconciliation_pre_submit_error("candidate fingerprint is missing"))?
        .to_string();
    let repository = RepositoryIdentity::gwt_upstream();
    let public_context = PublicMutationContext::for_repo_with_deadline(env.repo_path(), deadline);

    for duplicate in owners.iter().skip(1) {
        let duplicate_number = duplicate.number;
        verify_reconciliation_pair(env, &canonical, duplicate, &fingerprint, deadline)?;
        let payload = render_reconciliation_comment_payload(
            candidate,
            canonical_number,
            duplicate_number,
            &public_context,
        )
        .map_err(|error| {
            reconciliation_pre_submit_error(&format!(
                "duplicate reconciliation privacy validation failed: {error}"
            ))
        })?;
        let comments = comments_by_owner
            .remove(&IssueNumber(duplicate_number))
            .unwrap_or_default();
        if !comments.iter().any(|comment| {
            comment_matches_reconciliation(
                comment,
                canonical_number,
                duplicate_number,
                &fingerprint,
            )
        }) {
            let intent = ResolutionAttemptIntent::ReconciliationComment {
                canonical_owner_number: canonical_number,
                duplicate_owner_number: duplicate_number,
                public_payload_digest: public_payload_digest(
                    "gwt.improvement.reconciliation-comment-intent.v1",
                    &[&payload.body],
                ),
            };
            mark_resolution_attempt_submitted(env.repo_path(), candidate, token, intent.clone())?;
            let created = env
                .improvement_owner_client(deadline)
                .map_err(reconciliation_pre_submit_error_from_api)?
                .create_owner_comment(
                    &repository,
                    IssueNumber(duplicate_number),
                    &payload.body,
                    deadline,
                )
                .map_err(reconciliation_mutation_error)?;
            if created.body != payload.body {
                return Err(reconciliation_remote_error(
                    "duplicate reconciliation comment readback changed",
                ));
            }
            let readback = env
                .improvement_owner_client(deadline)
                .map_err(|error| reconciliation_remote_error(&error.to_string()))?
                .list_comments(&repository, IssueNumber(duplicate_number), deadline)
                .map_err(|error| reconciliation_remote_error(&error.to_string()))?;
            if !readback
                .items()
                .iter()
                .any(|comment| comment.body == payload.body)
            {
                return Err(reconciliation_remote_error(
                    "duplicate reconciliation comment is absent from authoritative readback",
                ));
            }
            complete_resolution_attempt_step(env.repo_path(), candidate, token, &intent)?;
        }

        verify_reconciliation_pair(env, &canonical, duplicate, &fingerprint, deadline)?;
        let close_intent = ResolutionAttemptIntent::CloseDuplicate {
            canonical_owner_number: canonical_number,
            duplicate_owner_number: duplicate_number,
        };
        mark_resolution_attempt_submitted(env.repo_path(), candidate, token, close_intent.clone())?;
        let closed = env
            .improvement_owner_client(deadline)
            .map_err(reconciliation_pre_submit_error_from_api)?
            .close_issue_verified(&repository, IssueNumber(duplicate_number), deadline)
            .map_err(reconciliation_mutation_error)?;
        if closed.repository != repository
            || closed.number != IssueNumber(duplicate_number)
            || closed.state != IssueState::Closed
            || closed.kind != RepositoryIssueKind::Plain
        {
            return Err(reconciliation_remote_error(
                "duplicate close readback did not verify the exact plain Issue",
            ));
        }
        complete_resolution_attempt_step(env.repo_path(), candidate, token, &close_intent)?;
    }

    let inspection = env
        .improvement_owner_client(deadline)
        .map_err(reconciliation_pre_submit_error_from_api)
        .and_then(|client| {
            inspect_owner_corpus(
                client,
                candidate,
                &ContractRoutingRegistry::current(),
                &NoSemanticOwnerAdvisor,
                deadline,
            )
            .map_err(|failure| {
                reconciliation_pre_submit_error(&format!(
                    "final duplicate reconciliation refresh failed: {}",
                    blocked_reason_token(failure.reason)
                ))
            })
        })?;
    match inspection {
        OwnerInspection::Active {
            owner, comments, ..
        } if owner.number == canonical_number => Ok((owner, comments)),
        _ => Err(reconciliation_pre_submit_error(
            "final duplicate reconciliation refresh did not elect only the lowest owner",
        )),
    }
}

fn verify_reconciliation_pair<E: CliEnv>(
    env: &mut E,
    canonical: &OwnerCandidate,
    duplicate: &OwnerCandidate,
    fingerprint: &str,
    deadline: &ResolutionDeadline,
) -> Result<(), OwnerResolutionCommitError> {
    let repository = RepositoryIdentity::gwt_upstream();
    let client = env
        .improvement_owner_client(deadline)
        .map_err(reconciliation_pre_submit_error_from_api)?;
    let canonical_readback = client
        .fetch_issue(&repository, IssueNumber(canonical.number), deadline)
        .map_err(reconciliation_pre_submit_error_from_api)?;
    validate_active_owner_readback(canonical, fingerprint, &canonical_readback)
        .map_err(|error| reconciliation_pre_submit_error(&error.to_string()))?;
    let duplicate_readback = client
        .fetch_issue(&repository, IssueNumber(duplicate.number), deadline)
        .map_err(reconciliation_pre_submit_error_from_api)?;
    validate_active_owner_readback(duplicate, fingerprint, &duplicate_readback)
        .map_err(|error| reconciliation_pre_submit_error(&error.to_string()))
}

fn create_owner_with_postflight<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    authorization: &ProvenZeroAuthorization,
    deadline: &ResolutionDeadline,
) -> Result<NewOwnerPostflight, OwnerResolutionCommitError> {
    let created = create_new_owner_mutation(env, candidate, authorization, deadline)
        .map_err(clear_create_root_on_pre_submit)?;
    settle_created_owner_postflight(env, candidate, token, created, None, deadline)
}

fn create_regression_owner_with_postflight<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    authorization: &ProvenRegressionAuthorization,
    deadline: &ResolutionDeadline,
) -> Result<NewOwnerPostflight, OwnerResolutionCommitError> {
    let created = create_regression_owner_mutation(env, candidate, authorization, deadline)
        .map_err(clear_create_root_on_pre_submit)?;
    settle_created_owner_postflight(
        env,
        candidate,
        token,
        created,
        Some(&authorization.recurrence_occurrence_keys),
        deadline,
    )
}

fn settle_created_owner_postflight<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    created: CreatedOwnerMutation,
    occurrence_keys: Option<&[String]>,
    deadline: &ResolutionDeadline,
) -> Result<NewOwnerPostflight, OwnerResolutionCommitError> {
    let observed_create_intent = record_created_owner_readback(
        env.repo_path(),
        candidate,
        token,
        created.readback.number.0,
    )?;
    let inspection = env
        .improvement_owner_client(deadline)
        .map_err(|error| {
            restore_remote_intent(candidate, &observed_create_intent);
            remote_commit_error(
                BlockedReason::Readback,
                "REFRESH_OWNER_CORPUS",
                error.into(),
            )
        })
        .and_then(|client| {
            inspect_owner_corpus(
                client,
                candidate,
                &ContractRoutingRegistry::current(),
                &NoSemanticOwnerAdvisor,
                deadline,
            )
            .map_err(|failure| {
                restore_remote_intent(candidate, &observed_create_intent);
                remote_commit_error(
                    failure.reason,
                    failure.remediation,
                    owner_projection_error(&format!(
                        "post-create owner refresh failed: {}",
                        blocked_reason_token(failure.reason)
                    )),
                )
            })
        })?;
    let (owner, comments) = match inspection {
        OwnerInspection::Active {
            owner, comments, ..
        } if owner.number == created.readback.number.0 => (owner, comments),
        OwnerInspection::Active { owner, .. } => {
            arm_reconciliation_required(
                env.repo_path(),
                candidate,
                token,
                &[owner.number, created.readback.number.0],
            )
            .map_err(|error| {
                remote_commit_error(
                    BlockedReason::Reconciliation,
                    "RECONCILE_DUPLICATE_OWNERS",
                    error,
                )
            })?;
            return Err(remote_commit_error(
                BlockedReason::Reconciliation,
                "RECONCILE_DUPLICATE_OWNERS",
                owner_projection_error(
                    "post-create owner refresh did not include the read-back created owner",
                ),
            ));
        }
        OwnerInspection::DuplicateExactIssues {
            owners,
            comments_by_owner,
            ..
        } => {
            let mut reconciliation_owner_numbers =
                owners.iter().map(|owner| owner.number).collect::<Vec<_>>();
            reconciliation_owner_numbers.push(created.readback.number.0);
            reconciliation_owner_numbers.sort_unstable();
            reconciliation_owner_numbers.dedup();
            arm_reconciliation_required(
                env.repo_path(),
                candidate,
                token,
                &reconciliation_owner_numbers,
            )
            .map_err(|error| {
                remote_commit_error(
                    BlockedReason::Reconciliation,
                    "RECONCILE_DUPLICATE_OWNERS",
                    error,
                )
            })?;
            complete_resolution_attempt_step(
                env.repo_path(),
                candidate,
                token,
                &observed_create_intent,
            )?;
            let reconciled = reconcile_duplicate_owners(
                env,
                candidate,
                token,
                owners,
                comments_by_owner,
                deadline,
            )?;
            if reconciled.0.number == created.readback.number.0 {
                mark_resolution_attempt_submitted(
                    env.repo_path(),
                    candidate,
                    token,
                    observed_create_intent.clone(),
                )
                .map_err(|error| {
                    remote_commit_error(
                        BlockedReason::LocalCommit,
                        "REPAIR_OWNER_PROJECTION",
                        owner_projection_error(&format!(
                            "failed to preserve created-owner recovery intent: {error}"
                        )),
                    )
                })?;
            }
            reconciled
        }
        OwnerInspection::Ambiguous {
            owner_candidates, ..
        } => {
            let mut reconciliation_owner_numbers = owner_candidates
                .iter()
                .map(|owner| owner.number)
                .collect::<Vec<_>>();
            reconciliation_owner_numbers.push(created.readback.number.0);
            reconciliation_owner_numbers.sort_unstable();
            reconciliation_owner_numbers.dedup();
            if reconciliation_owner_numbers.len() > 1 {
                arm_reconciliation_required(
                    env.repo_path(),
                    candidate,
                    token,
                    &reconciliation_owner_numbers,
                )
                .map_err(|error| {
                    remote_commit_error(
                        BlockedReason::Reconciliation,
                        "RECONCILE_DUPLICATE_OWNERS",
                        error,
                    )
                })?;
            }
            restore_remote_intent(candidate, &observed_create_intent);
            return Err(remote_commit_error(
                BlockedReason::Reconciliation,
                "RECONCILE_DUPLICATE_OWNERS",
                owner_projection_error(
                    "post-create owner refresh returned ambiguous active owners",
                ),
            ));
        }
        _ => {
            restore_remote_intent(candidate, &observed_create_intent);
            return Err(remote_commit_error(
                BlockedReason::Reconciliation,
                "REFRESH_OWNER_CORPUS",
                owner_projection_error(
                    "post-create owner refresh did not return one active canonical owner",
                ),
            ));
        }
    };
    if owner.number == created.readback.number.0 {
        if let Some(occurrence_keys) = occurrence_keys {
            commit_created_owner_readback_for_occurrences(
                env,
                candidate,
                &created.readback,
                &created.input,
                &created.fingerprint,
                CreatedOwnerOccurrenceCommit {
                    occurrence_keys,
                    attempt_token: Some(token),
                },
                deadline,
            )?;
        } else {
            commit_created_owner_readback(
                env,
                candidate,
                &created.readback,
                &created.input,
                &created.fingerprint,
                deadline,
            )?;
        }
        Ok(NewOwnerPostflight::Committed)
    } else {
        Ok(NewOwnerPostflight::Active { owner, comments })
    }
}

fn restore_remote_intent(candidate: &mut ImprovementCandidate, intent: &ResolutionAttemptIntent) {
    if let Some(attempt) = candidate.attempt.as_mut() {
        attempt.remote_phase = AttemptRemotePhase::Submitted;
        attempt.remote_mutation_seen = true;
        attempt.intent = intent.clone();
        attempt.expires_at = Utc::now();
    }
}

fn complete_resolution_attempt_step(
    repo_root: &Path,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    intent: &ResolutionAttemptIntent,
) -> Result<(), OwnerResolutionCommitError> {
    let attempt = super::improvement_store::update(repo_root, |store| {
        super::improvement_store::complete_attempt_step(
            store,
            &token.candidate_id,
            &token.attempt_id,
            intent,
        )
    })
    .map_err(|error| {
        remote_commit_error(
            BlockedReason::Reconciliation,
            "RECONCILE_DUPLICATE_OWNERS",
            error,
        )
    })?;
    candidate.attempt = Some(attempt);
    Ok(())
}

fn complete_occurrence_comment_step(
    repo_root: &Path,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    intent: &ResolutionAttemptIntent,
) -> Result<(), OwnerResolutionCommitError> {
    let attempt = super::improvement_store::update(repo_root, |store| {
        super::improvement_store::complete_attempt_step(
            store,
            &token.candidate_id,
            &token.attempt_id,
            intent,
        )
    })
    .map_err(|error| {
        remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error)
    })?;
    candidate.attempt = Some(attempt);
    Ok(())
}

fn adopt_recovered_reconciliation_step<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    recovery_intent: &ResolutionAttemptIntent,
    deadline: &ResolutionDeadline,
) -> Result<Option<(IssueNumber, RepositoryComment)>, OwnerResolutionCommitError> {
    let repository = RepositoryIdentity::gwt_upstream();
    match recovery_intent {
        ResolutionAttemptIntent::ReconciliationComment {
            canonical_owner_number,
            duplicate_owner_number,
            public_payload_digest: expected_digest,
        } => {
            let fingerprint = candidate
                .fingerprint
                .as_deref()
                .ok_or_else(|| reconciliation_remote_error("candidate fingerprint is missing"))?;
            let comments = env
                .improvement_owner_client(deadline)
                .map_err(|error| {
                    remote_commit_error(
                        BlockedReason::Reconciliation,
                        "RECONCILE_DUPLICATE_OWNERS",
                        error.into(),
                    )
                })?
                .list_comments(&repository, IssueNumber(*duplicate_owner_number), deadline)
                .map_err(|error| {
                    remote_commit_error(
                        BlockedReason::Reconciliation,
                        "RECONCILE_DUPLICATE_OWNERS",
                        error.into(),
                    )
                })?;
            let submitted_comment = comments
                .items()
                .iter()
                .find(|comment| {
                    comment_matches_reconciliation(
                        comment,
                        *canonical_owner_number,
                        *duplicate_owner_number,
                        fingerprint,
                    ) && public_payload_digest(
                        "gwt.improvement.reconciliation-comment-intent.v1",
                        &[&comment.body],
                    ) == *expected_digest
                })
                .cloned()
                .ok_or_else(|| {
                    reconciliation_remote_error(
                        "submitted reconciliation comment is absent from authoritative readback",
                    )
                })?;
            complete_resolution_attempt_step(env.repo_path(), candidate, token, recovery_intent)?;
            Ok(Some((
                IssueNumber(*duplicate_owner_number),
                submitted_comment,
            )))
        }
        ResolutionAttemptIntent::CloseDuplicate {
            duplicate_owner_number,
            ..
        } => {
            let readback = env
                .improvement_owner_client(deadline)
                .map_err(|error| {
                    remote_commit_error(
                        BlockedReason::Reconciliation,
                        "RECONCILE_DUPLICATE_OWNERS",
                        error.into(),
                    )
                })?
                .fetch_issue(&repository, IssueNumber(*duplicate_owner_number), deadline)
                .map_err(|error| {
                    remote_commit_error(
                        BlockedReason::Reconciliation,
                        "RECONCILE_DUPLICATE_OWNERS",
                        error.into(),
                    )
                })?;
            let fingerprint = candidate.fingerprint.as_deref().ok_or_else(|| {
                remote_commit_error(
                    BlockedReason::Reconciliation,
                    "RECONCILE_DUPLICATE_OWNERS",
                    owner_projection_error("candidate fingerprint is missing"),
                )
            })?;
            if readback.repository != repository
                || readback.number != IssueNumber(*duplicate_owner_number)
                || readback.kind != RepositoryIssueKind::Plain
                || !exact_fingerprint_markers(&readback.body)
                    .iter()
                    .any(|marker| marker == fingerprint)
            {
                return Err(remote_commit_error(
                    BlockedReason::Reconciliation,
                    "RECONCILE_DUPLICATE_OWNERS",
                    owner_projection_error(
                        "submitted duplicate close could not be matched to authoritative readback",
                    ),
                ));
            }
            complete_resolution_attempt_step(env.repo_path(), candidate, token, recovery_intent)?;
            Ok(None)
        }
        _ => Ok(None),
    }
}

fn reconciliation_pre_submit_error(message: &str) -> OwnerResolutionCommitError {
    pre_submit_commit_error(
        BlockedReason::Reconciliation,
        "RECONCILE_DUPLICATE_OWNERS",
        owner_projection_error(message),
    )
}

fn reconciliation_pre_submit_error_from_api(error: GitHubApiError) -> OwnerResolutionCommitError {
    pre_submit_commit_error(
        BlockedReason::Reconciliation,
        "RECONCILE_DUPLICATE_OWNERS",
        error.into(),
    )
}

fn reconciliation_remote_error(message: &str) -> OwnerResolutionCommitError {
    remote_commit_error(
        BlockedReason::Reconciliation,
        "RECONCILE_DUPLICATE_OWNERS",
        owner_projection_error(message),
    )
}

fn reconciliation_mutation_error(error: OwnerMutationError) -> OwnerResolutionCommitError {
    match error {
        OwnerMutationError::PreSubmit(error) => reconciliation_pre_submit_error_from_api(error),
        OwnerMutationError::RemoteOutcomeUnknown(error) => remote_commit_error(
            BlockedReason::Reconciliation,
            "RECONCILE_DUPLICATE_OWNERS",
            error.into(),
        ),
    }
}

fn settle_non_owner_preflight<E: CliEnv>(
    env: &mut E,
    mut candidate: ImprovementCandidate,
    token: &ResolutionAttemptToken,
    outcome: OwnerPreflightOutcome,
    recovery_intent: Option<&ResolutionAttemptIntent>,
    reconciliation_only: bool,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    let failure = match outcome {
        OwnerPreflightOutcome::Historical { .. } => OwnerResolutionFailure {
            reason: BlockedReason::Ambiguity,
            failure_subcode: None,
            remediation: "VERIFY_HISTORICAL_RECURRENCE",
        },
        OwnerPreflightOutcome::RemoteOutcomeUnknown {
            failure_reason,
            failure_subcode,
        }
        | OwnerPreflightOutcome::Blocked {
            reason: failure_reason,
            failure_subcode,
        } => OwnerResolutionFailure {
            reason: failure_reason,
            failure_subcode,
            remediation: "RETRY_OWNER_RESOLUTION",
        },
        OwnerPreflightOutcome::Active { .. }
        | OwnerPreflightOutcome::RegressionCreateAuthorized { .. }
        | OwnerPreflightOutcome::CreateAuthorized { .. }
        | OwnerPreflightOutcome::DuplicateExactIssues { .. } => {
            return Err(owner_projection_error(
                "owner preflight outcome was routed to the wrong settlement path",
            ));
        }
    };
    if let Some(recovery_intent) = recovery_intent {
        return preserve_remote_unknown_recovery(env, candidate, token, recovery_intent, failure);
    }
    if reconciliation_only {
        let failure = OwnerResolutionFailure {
            reason: BlockedReason::Reconciliation,
            failure_subcode: failure.failure_subcode,
            remediation: "RECONCILE_DUPLICATE_OWNERS",
        };
        block_owner_resolution(env, &mut candidate, failure, None, false)?;
        candidate.attempt = None;
        return persist_and_post_resolver_failure(env, candidate, token, failure);
    }
    if candidate.state != CandidateState::Blocked {
        block_owner_resolution(env, &mut candidate, failure, None, false)?;
    }
    candidate.attempt = None;
    persist_and_post_resolver_failure(env, candidate, token, failure)
}

fn preserve_remote_unknown_recovery<E: CliEnv>(
    env: &mut E,
    mut candidate: ImprovementCandidate,
    token: &ResolutionAttemptToken,
    recovery_intent: &ResolutionAttemptIntent,
    failure: OwnerResolutionFailure,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    if candidate.state == CandidateState::Blocked {
        transition_candidate(&mut candidate, CandidateState::OwnerResolving)?;
    }
    let now = Utc::now();
    candidate.blocked_reason = None;
    candidate.failure_subcode = None;
    candidate.retry = Some(RetryMetadata {
        retryable: true,
        remediation: "REFRESH_OWNER_CORPUS".to_string(),
        failed_at: now.to_rfc3339(),
    });
    candidate.resolver_snapshot = None;
    candidate.updated_at = now.to_rfc3339();
    if let Some(attempt) = candidate.attempt.as_mut() {
        attempt.remote_phase = AttemptRemotePhase::Submitted;
        attempt.remote_mutation_seen = true;
        attempt.intent = recovery_intent.clone();
        attempt.expires_at = now;
    }
    transition_candidate(&mut candidate, CandidateState::RemoteOutcomeUnknown)?;
    persist_and_post_resolver_failure(env, candidate, token, failure)
}

fn begin_resolution_attempt(
    repo_root: &Path,
    candidate_id: &str,
    budget_profile: CaptureBudgetProfile,
) -> Result<ResolutionAttemptStart, gwt_github::SpecOpsError> {
    let lease_owner = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("pid-{}", std::process::id()));
    let now = Utc::now();
    let ttl = budget_profile.lease_ttl();
    super::improvement_store::update(repo_root, |store| {
        let index = store
            .candidates
            .iter()
            .position(|candidate| candidate.id == candidate_id)
            .ok_or_else(|| owner_projection_error("candidate not found"))?;
        let initial_state = store.candidates[index].state;
        match initial_state {
            CandidateState::Linked | CandidateState::Created => {
                transition_candidate(&mut store.candidates[index], CandidateState::Recurrent)?;
                transition_candidate(&mut store.candidates[index], CandidateState::OwnerResolving)?;
            }
            CandidateState::Blocked
            | CandidateState::OwnerResolving
            | CandidateState::RemoteOutcomeUnknown
            | CandidateState::Recurrent => {}
            CandidateState::NeedsEvidence
            | CandidateState::Pending
            | CandidateState::Parked
            | CandidateState::Dismissed => {
                return Err(owner_projection_error(
                    "candidate is not eligible for Owner Resolution",
                ));
            }
        }

        let reconciliation_only = store.candidates[index].reconciliation_required
            || (initial_state == CandidateState::Blocked
                && store.candidates[index].blocked_reason == Some(BlockedReason::Reconciliation))
            || store.candidates[index]
                .attempt
                .as_ref()
                .is_some_and(|attempt| {
                    matches!(
                        attempt.intent,
                        ResolutionAttemptIntent::ReconciliationComment { .. }
                            | ResolutionAttemptIntent::CloseDuplicate { .. }
                    )
                });
        if reconciliation_only {
            store.candidates[index].reconciliation_required = true;
        }
        let submitted_recovery_intent = store.candidates[index]
            .attempt
            .as_ref()
            .filter(|attempt| attempt.remote_phase == AttemptRemotePhase::Submitted)
            .map(|attempt| attempt.intent.clone());
        let pending_create_recovery_intent =
            store.candidates[index].pending_create_resolution.clone();
        let mut recovering_remote_unknown = initial_state == CandidateState::RemoteOutcomeUnknown
            || (submitted_recovery_intent.is_none() && pending_create_recovery_intent.is_some());
        let mut recovery_intent = submitted_recovery_intent
            .or(pending_create_recovery_intent)
            .unwrap_or_default();
        let decision = super::improvement_store::acquire_attempt_lease(
            store,
            candidate_id,
            &lease_owner,
            now,
            ttl,
        )?;
        let lease = match decision {
            AttemptLeaseDecision::Acquired(lease) => lease,
            AttemptLeaseDecision::Busy {
                attempt_id,
                lease_owner,
                expires_at,
            } => {
                return Err(owner_projection_error(&format!(
                    "owner resolution attempt is already in progress: {attempt_id} by {lease_owner} until {expires_at}"
                )));
            }
            AttemptLeaseDecision::RemoteOutcomeUnknown => {
                recovering_remote_unknown = true;
                recovery_intent = store.candidates[index]
                    .attempt
                    .as_ref()
                    .map(|attempt| attempt.intent.clone())
                    .unwrap_or_default();
                store.candidates[index].attempt = None;
                transition_candidate(&mut store.candidates[index], CandidateState::OwnerResolving)?;
                match super::improvement_store::acquire_attempt_lease(
                    store,
                    candidate_id,
                    &lease_owner,
                    now,
                    ttl,
                )? {
                    AttemptLeaseDecision::Acquired(lease) => lease,
                    _ => {
                        return Err(owner_projection_error(
                            "failed to acquire reconciliation attempt lease",
                        ));
                    }
                }
            }
        };
        if recovering_remote_unknown && recovery_intent != ResolutionAttemptIntent::Unassigned {
            super::improvement_store::mark_attempt_submitted(
                store,
                candidate_id,
                &lease.attempt_id,
                recovery_intent.clone(),
            )?;
        }
        if matches!(
            store.candidates[index].state,
            CandidateState::Blocked
                | CandidateState::RemoteOutcomeUnknown
                | CandidateState::Recurrent
        ) {
            transition_candidate(&mut store.candidates[index], CandidateState::OwnerResolving)?;
        }
        let candidate = store.candidates[index].clone();
        Ok(ResolutionAttemptStart::Acquired {
            token: ResolutionAttemptToken {
                candidate_id: candidate.id.clone(),
                attempt_id: lease.attempt_id,
                ttl,
            },
            candidate,
            recovering_remote_unknown,
            recovery_intent,
            reconciliation_only,
        })
    })
}

fn adopt_recovered_occurrence_comments_step<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    recovery_intent: &ResolutionAttemptIntent,
    deadline: &ResolutionDeadline,
) -> Result<Option<(IssueNumber, RepositoryComment)>, OwnerResolutionCommitError> {
    let ResolutionAttemptIntent::OccurrenceComments {
        owner_number,
        occurrence_keys,
        public_payload_digest: expected_digest,
    } = recovery_intent
    else {
        return Ok(None);
    };
    let repo_root = env.repo_path().to_path_buf();
    let fingerprint = candidate.fingerprint.as_deref().ok_or_else(|| {
        remote_commit_error(
            BlockedReason::Readback,
            "REFRESH_OWNER_CORPUS",
            owner_projection_error("candidate fingerprint is missing"),
        )
    })?;
    let repository = RepositoryIdentity::gwt_upstream();
    let client = env.improvement_owner_client(deadline).map_err(|error| {
        remote_commit_error(
            BlockedReason::Readback,
            "REFRESH_OWNER_CORPUS",
            error.into(),
        )
    })?;
    let owner_readback = client
        .fetch_issue(&repository, IssueNumber(*owner_number), deadline)
        .map_err(|error| {
            remote_commit_error(
                BlockedReason::Readback,
                "REFRESH_OWNER_CORPUS",
                error.into(),
            )
        })?;
    if owner_readback.repository != repository
        || owner_readback.number != IssueNumber(*owner_number)
        || !matches!(
            owner_readback.kind,
            RepositoryIssueKind::Plain | RepositoryIssueKind::Spec
        )
    {
        return Err(remote_commit_error(
            BlockedReason::Readback,
            "REFRESH_OWNER_CORPUS",
            owner_projection_error(
                "submitted occurrence owner did not match authoritative readback",
            ),
        ));
    }
    let comments = client
        .list_comments(&repository, IssueNumber(*owner_number), deadline)
        .map_err(|error| {
            remote_commit_error(
                BlockedReason::Readback,
                "REFRESH_OWNER_CORPUS",
                error.into(),
            )
        })?;
    let [occurrence_key] = occurrence_keys.as_slice() else {
        return Err(remote_commit_error(
            BlockedReason::Readback,
            "REFRESH_OWNER_CORPUS",
            owner_projection_error(
                "submitted occurrence recovery requires exactly one immutable comment intent",
            ),
        ));
    };
    if !candidate
        .distinct_occurrences
        .iter()
        .any(|occurrence| occurrence.opaque_key == *occurrence_key)
    {
        return Err(remote_commit_error(
            BlockedReason::Readback,
            "REFRESH_OWNER_CORPUS",
            owner_projection_error("submitted occurrence comment key is absent from the candidate"),
        ));
    }
    let submitted_comment = comments
        .items()
        .iter()
        .find(|comment| {
            comment_matches_occurrence(comment, occurrence_key, fingerprint)
                && public_payload_digest(
                    "gwt.improvement.owner-comment-intent.v1",
                    &[occurrence_key, &comment.body],
                ) == *expected_digest
        })
        .cloned()
        .ok_or_else(|| {
            remote_commit_error(
                BlockedReason::Readback,
                "REFRESH_OWNER_CORPUS",
                owner_projection_error(
                    "submitted occurrence comment is absent from authoritative readback",
                ),
            )
        })?;

    let attempt = super::improvement_store::update(&repo_root, |store| {
        super::improvement_store::complete_attempt_step(
            store,
            &token.candidate_id,
            &token.attempt_id,
            recovery_intent,
        )
    })
    .map_err(|error| {
        remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error)
    })?;
    candidate.attempt = Some(attempt);
    Ok(Some((IssueNumber(*owner_number), submitted_comment)))
}

fn new_owner_attempt_intent(authorization: &ProvenZeroAuthorization) -> ResolutionAttemptIntent {
    ResolutionAttemptIntent::CreateIssue {
        fingerprint: authorization.fingerprint.clone(),
        public_payload_digest: public_payload_digest(
            "gwt.improvement.owner-create-intent.v1",
            &[&authorization.payload.title, &authorization.payload.body],
        ),
        created_owner_number: None,
    }
}

fn regression_owner_attempt_intent(
    authorization: &ProvenRegressionAuthorization,
) -> ResolutionAttemptIntent {
    ResolutionAttemptIntent::CreateRegressionIssue {
        fingerprint: authorization.fingerprint.clone(),
        historical_owner_number: authorization.historical_owner.number,
        recurrence_occurrence_keys: authorization.recurrence_occurrence_keys.clone(),
        recurrence_proof_digest: authorization.recurrence_proof_digest.clone(),
        public_payload_digest: public_payload_digest(
            "gwt.improvement.regression-create-intent.v1",
            &[&authorization.payload.title, &authorization.payload.body],
        ),
        created_owner_number: None,
    }
}

fn recorded_created_owner_number(intent: &ResolutionAttemptIntent) -> Option<u64> {
    match intent {
        ResolutionAttemptIntent::CreateIssue {
            created_owner_number,
            ..
        }
        | ResolutionAttemptIntent::CreateRegressionIssue {
            created_owner_number,
            ..
        } => *created_owner_number,
        _ => None,
    }
}

fn record_created_owner_readback(
    repo_root: &Path,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    owner_number: u64,
) -> Result<ResolutionAttemptIntent, OwnerResolutionCommitError> {
    let intent = record_local_created_owner_number(candidate, token, owner_number)?;
    let (attempt, pending_create_resolution) =
        super::improvement_store::update(repo_root, |store| {
            super::improvement_store::renew_attempt_lease(
                store,
                &token.candidate_id,
                &token.attempt_id,
                Utc::now(),
                token.ttl,
            )?;
            let attempt = super::improvement_store::record_created_owner_number(
                store,
                &token.candidate_id,
                &token.attempt_id,
                owner_number,
            )?;
            let pending_create_resolution = store
                .candidates
                .iter()
                .find(|stored| stored.id == token.candidate_id)
                .and_then(|stored| stored.pending_create_resolution.clone())
                .ok_or_else(|| owner_projection_error("pending create resolution is missing"))?;
            Ok((attempt, Some(pending_create_resolution)))
        })
        .map_err(|error| {
            remote_commit_error(BlockedReason::LocalCommit, "REPAIR_OWNER_PROJECTION", error)
        })?;
    candidate.attempt = Some(attempt);
    candidate.pending_create_resolution = pending_create_resolution;
    Ok(candidate
        .pending_create_resolution
        .clone()
        .unwrap_or(intent))
}

fn record_local_created_owner_number(
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    owner_number: u64,
) -> Result<ResolutionAttemptIntent, OwnerResolutionCommitError> {
    let invalid_readback = |message| {
        remote_commit_error(
            BlockedReason::LocalCommit,
            "REPAIR_OWNER_PROJECTION",
            owner_projection_error(message),
        )
    };
    if owner_number == 0 {
        return Err(invalid_readback("created owner number must be positive"));
    }
    if token.candidate_id != candidate.id {
        return Err(invalid_readback(
            "owner resolution attempt token candidate mismatch",
        ));
    }
    let attempt = candidate
        .attempt
        .as_mut()
        .filter(|attempt| attempt.attempt_id == token.attempt_id)
        .ok_or_else(|| invalid_readback("owner resolution attempt lease is stale"))?;
    if attempt.remote_phase != AttemptRemotePhase::Submitted {
        return Err(invalid_readback(
            "created owner number requires a submitted create intent",
        ));
    }
    let original_intent = attempt.intent.clone();
    let created_owner_number = match &mut attempt.intent {
        ResolutionAttemptIntent::CreateIssue {
            created_owner_number,
            ..
        }
        | ResolutionAttemptIntent::CreateRegressionIssue {
            created_owner_number,
            ..
        } => created_owner_number,
        _ => {
            return Err(invalid_readback(
                "created owner number requires a submitted create intent",
            ));
        }
    };
    match created_owner_number {
        Some(existing) if *existing != owner_number => {
            return Err(invalid_readback(
                "created owner number cannot change after readback",
            ));
        }
        Some(_) => {}
        None => *created_owner_number = Some(owner_number),
    }
    let recorded_intent = attempt.intent.clone();
    match candidate.pending_create_resolution.as_mut() {
        Some(pending) if *pending == original_intent => *pending = recorded_intent.clone(),
        Some(pending) if *pending == recorded_intent => {}
        Some(_) => {
            return Err(invalid_readback(
                "submitted create intent conflicts with the pending create resolution",
            ));
        }
        None => candidate.pending_create_resolution = Some(recorded_intent.clone()),
    }
    Ok(recorded_intent)
}

fn public_payload_digest(domain: &str, fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    for value in std::iter::once(domain).chain(fields.iter().copied()) {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("sha256:{}", hex::encode(digest.finalize()))
}

fn mark_resolution_attempt_submitted(
    repo_root: &Path,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
    intent: ResolutionAttemptIntent,
) -> Result<(), OwnerResolutionCommitError> {
    if token.candidate_id != candidate.id {
        return Err(pre_submit_commit_error(
            BlockedReason::Store,
            "RELOAD_CANDIDATE_STORE",
            owner_projection_error("owner resolution attempt token candidate mismatch"),
        ));
    }
    let (attempt, pending_create_resolution) =
        super::improvement_store::update(repo_root, |store| {
            super::improvement_store::renew_attempt_lease(
                store,
                &token.candidate_id,
                &token.attempt_id,
                Utc::now(),
                token.ttl,
            )?;
            super::improvement_store::mark_attempt_submitted(
                store,
                &token.candidate_id,
                &token.attempt_id,
                intent.clone(),
            )?;
            let candidate = store
                .candidates
                .iter()
                .find(|stored| stored.id == token.candidate_id)
                .ok_or_else(|| owner_projection_error("submitted attempt candidate is missing"))?;
            let attempt = candidate
                .attempt
                .clone()
                .ok_or_else(|| owner_projection_error("submitted attempt lease is missing"))?;
            Ok((attempt, candidate.pending_create_resolution.clone()))
        })
        .map_err(|error| {
            pre_submit_commit_error(BlockedReason::Store, "RELOAD_CANDIDATE_STORE", error)
        })?;
    candidate.attempt = Some(attempt);
    candidate.pending_create_resolution = pending_create_resolution;
    Ok(())
}

fn renew_resolution_attempt(
    repo_root: &Path,
    candidate: &mut ImprovementCandidate,
    token: &ResolutionAttemptToken,
) -> Result<(), OwnerResolutionCommitError> {
    if token.candidate_id != candidate.id {
        return Err(pre_submit_commit_error(
            BlockedReason::Store,
            "RELOAD_CANDIDATE_STORE",
            owner_projection_error("owner resolution attempt token candidate mismatch"),
        ));
    }
    let attempt = super::improvement_store::update(repo_root, |store| {
        super::improvement_store::renew_attempt_lease(
            store,
            &token.candidate_id,
            &token.attempt_id,
            Utc::now(),
            token.ttl,
        )
    })
    .map_err(|error| {
        pre_submit_commit_error(BlockedReason::Store, "RELOAD_CANDIDATE_STORE", error)
    })?;
    candidate.attempt = Some(attempt);
    Ok(())
}

fn settle_owner_commit_error<E: CliEnv>(
    env: &mut E,
    mut candidate: ImprovementCandidate,
    token: &ResolutionAttemptToken,
    error: OwnerResolutionCommitError,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    match error {
        OwnerResolutionCommitError::PreSubmit {
            failure,
            source: _,
            clear_create_root,
        } => {
            if clear_create_root {
                let submitted_create_intent = candidate.attempt.as_ref().and_then(|attempt| {
                    (attempt.remote_phase == AttemptRemotePhase::Submitted
                        && matches!(
                            attempt.intent,
                            ResolutionAttemptIntent::CreateIssue { .. }
                                | ResolutionAttemptIntent::CreateRegressionIssue { .. }
                        )
                        && candidate.pending_create_resolution.as_ref() == Some(&attempt.intent))
                    .then(|| attempt.intent.clone())
                });
                if let Some(create_intent) = submitted_create_intent {
                    super::improvement_store::clear_pending_create_resolution(
                        &mut candidate,
                        &create_intent,
                    )?;
                }
            }
            block_owner_resolution(env, &mut candidate, failure, None, false)?;
            candidate.attempt = None;
            persist_and_post_resolver_failure(env, candidate, token, failure)
        }
        OwnerResolutionCommitError::RemoteOutcomeUnknown { failure, source: _ } => {
            if let Some(attempt) = candidate.attempt.as_mut() {
                attempt.remote_phase = AttemptRemotePhase::Submitted;
                attempt.remote_mutation_seen = true;
                attempt.expires_at = Utc::now();
            }
            candidate.blocked_reason = None;
            candidate.failure_subcode = None;
            candidate.retry = Some(RetryMetadata {
                retryable: true,
                remediation: "REFRESH_OWNER_CORPUS".to_string(),
                failed_at: Utc::now().to_rfc3339(),
            });
            candidate.resolver_snapshot = None;
            candidate.updated_at = Utc::now().to_rfc3339();
            transition_candidate(&mut candidate, CandidateState::RemoteOutcomeUnknown)?;
            persist_and_post_resolver_failure(env, candidate, token, failure)
        }
        OwnerResolutionCommitError::DurableStatus(source) => Err(source),
    }
}

fn persist_and_post_resolver_failure<E: CliEnv>(
    env: &mut E,
    candidate: ImprovementCandidate,
    token: &ResolutionAttemptToken,
    failure: OwnerResolutionFailure,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    let repo_root = env.repo_path().to_path_buf();
    let persisted = persist_resolver_candidate(&repo_root, &candidate, token)?;
    post_persisted_resolver_failure_status(env, &persisted, failure)?;
    Ok(persisted)
}

fn persist_resolver_candidate(
    repo_root: &Path,
    candidate: &ImprovementCandidate,
    token: &ResolutionAttemptToken,
) -> Result<ImprovementCandidate, gwt_github::SpecOpsError> {
    super::improvement_store::update(repo_root, |store| {
        let stored = store
            .candidates
            .iter_mut()
            .find(|stored| stored.id == candidate.id)
            .ok_or_else(|| owner_projection_error("candidate not found"))?;
        if stored.fingerprint != candidate.fingerprint
            || stored.distinct_occurrences != candidate.distinct_occurrences
            || stored.state != CandidateState::OwnerResolving
            || stored.dismissed_reason != candidate.dismissed_reason
            || stored.audit != candidate.audit
        {
            return Err(owner_projection_error(
                "candidate changed while Owner Resolution was running",
            ));
        }
        if token.candidate_id != candidate.id
            || stored
                .attempt
                .as_ref()
                .is_none_or(|attempt| attempt.attempt_id != token.attempt_id)
        {
            return Err(owner_projection_error(
                "owner resolution attempt lease is stale",
            ));
        }
        stored.state = candidate.state;
        stored.blocked_reason = candidate.blocked_reason;
        stored.failure_subcode = candidate.failure_subcode;
        stored.retry = candidate.retry.clone();
        stored.resolver_snapshot = candidate.resolver_snapshot.clone();
        stored.reconciliation_required = candidate.reconciliation_required;
        stored.reconciliation_owner_numbers = candidate.reconciliation_owner_numbers.clone();
        stored.pending_create_resolution = candidate.pending_create_resolution.clone();
        stored.attempt = candidate.attempt.clone();
        stored.updated_at = candidate.updated_at.clone();
        Ok(stored.clone())
    })
}

fn post_persisted_resolver_failure_status<E: CliEnv>(
    env: &mut E,
    candidate: &ImprovementCandidate,
    failure: OwnerResolutionFailure,
) -> Result<(), gwt_github::SpecOpsError> {
    let subcode = failure
        .failure_subcode
        .map(failure_subcode_token)
        .unwrap_or("none");
    if candidate.state == CandidateState::RemoteOutcomeUnknown {
        return post_improvement_board_status(
            env,
            format!(
                "Current state: Improvement Candidate {id} is remote-outcome-unknown.\n\nReason: a submitted owner mutation could not be settled after {reason}/{subcode}.\n\nNext: REFRESH_OWNER_CORPUS before any mutation retry.",
                id = candidate.id,
                reason = blocked_reason_token(failure.reason),
            ),
        );
    }
    let remediation = candidate
        .retry
        .as_ref()
        .map(|retry| retry.remediation.as_str())
        .unwrap_or(failure.remediation);
    post_improvement_board_status(
        env,
        format!(
            "Current state: Improvement Candidate {id} owner resolution is blocked.\n\nReason: {reason}/{subcode}.\n\nNext: {remediation}.",
            id = candidate.id,
            reason = blocked_reason_token(failure.reason),
        ),
    )
}

fn validate_source_occurrence_snapshot(
    repo_root: &Path,
    candidate: &ImprovementCandidate,
    occurrences: &[super::improvement::DistinctOccurrence],
) -> Result<(), gwt_github::SpecOpsError> {
    let source_store = super::improvement_store::load_and_repair(repo_root)?;
    let source_candidate = source_store
        .candidates
        .iter()
        .find(|stored| stored.id == candidate.id)
        .ok_or_else(|| owner_projection_error("source candidate is missing"))?;
    if source_candidate.fingerprint != candidate.fingerprint
        || source_candidate.distinct_occurrences != occurrences
    {
        return Err(owner_projection_error(
            "source candidate projection coverage changed before remote mutation",
        ));
    }
    Ok(())
}

fn validate_active_owner_readback(
    owner: &OwnerCandidate,
    fingerprint: &str,
    readback: &RepositoryIssue,
) -> Result<(), gwt_github::SpecOpsError> {
    let expected_kind = match owner.kind {
        OwnerKind::Issue => RepositoryIssueKind::Plain,
        OwnerKind::Spec => RepositoryIssueKind::Spec,
    };
    if readback.repository != RepositoryIdentity::gwt_upstream()
        || readback.number != IssueNumber(owner.number)
        || readback.state != IssueState::Open
        || readback.kind != expected_kind
    {
        return Err(owner_projection_error(
            "active owner readback no longer matches the selected owner",
        ));
    }
    if owner.match_basis == OwnerMatchBasis::Fingerprint
        && !exact_fingerprint_markers(&readback.body)
            .iter()
            .any(|marker| marker == fingerprint)
    {
        return Err(owner_projection_error(
            "active owner readback no longer contains the exact fingerprint",
        ));
    }
    Ok(())
}

fn durable_owner_from_readback(
    readback: &RepositoryIssue,
    fingerprint: &str,
) -> DurableOwnerSnapshot {
    DurableOwnerSnapshot {
        number: readback.number.0,
        kind: match readback.kind {
            RepositoryIssueKind::Plain => OwnerKind::Issue,
            RepositoryIssueKind::Spec => OwnerKind::Spec,
        },
        title: readback.title.clone(),
        active: true,
        url: format!(
            "https://github.com/akiojin/gwt/issues/{}",
            readback.number.0
        ),
        fingerprint: fingerprint.to_string(),
        readback_verified_at: Utc::now().to_rfc3339(),
    }
}

fn active_api_commit_error(
    error: GitHubApiError,
    remote_mutation_seen: bool,
) -> OwnerResolutionCommitError {
    if !remote_mutation_seen {
        return pre_submit_commit_error_from_api(error);
    }
    let failure = owner_failure_from_api(&error);
    OwnerResolutionCommitError::RemoteOutcomeUnknown {
        failure,
        source: error.into(),
    }
}

fn active_mutation_commit_error(
    error: OwnerMutationError,
    remote_mutation_seen: bool,
) -> OwnerResolutionCommitError {
    match error {
        OwnerMutationError::PreSubmit(error) if !remote_mutation_seen => {
            pre_submit_commit_error_from_api(error)
        }
        OwnerMutationError::PreSubmit(error) | OwnerMutationError::RemoteOutcomeUnknown(error) => {
            let failure = owner_failure_from_api(&error);
            OwnerResolutionCommitError::RemoteOutcomeUnknown {
                failure,
                source: error.into(),
            }
        }
    }
}

fn active_spec_commit_error(
    error: gwt_github::SpecOpsError,
    remote_mutation_seen: bool,
    reason: BlockedReason,
    remediation: &'static str,
) -> OwnerResolutionCommitError {
    if remote_mutation_seen {
        remote_commit_error(reason, remediation, error)
    } else {
        pre_submit_commit_error(reason, remediation, error)
    }
}

fn post_active_owner_linked_status<E: CliEnv>(
    env: &mut E,
    candidate: &ImprovementCandidate,
    owner_number: u64,
) -> Result<(), gwt_github::SpecOpsError> {
    post_improvement_board_status(
        env,
        format!(
            "Current state: Improvement Candidate {id} was linked to active akiojin/gwt owner #{owner_number}.\n\nReason: the authoritative owner and occurrence marker were read back before the projection-first local commit.\n\nNext: Track the owner at https://github.com/akiojin/gwt/issues/{owner_number}.",
            id = candidate.id,
        ),
    )
}

fn owner_candidate(issue: &RepositoryIssue, match_basis: OwnerMatchBasis) -> OwnerCandidate {
    OwnerCandidate {
        number: issue.number.0,
        kind: match issue.kind {
            RepositoryIssueKind::Plain => OwnerKind::Issue,
            RepositoryIssueKind::Spec => OwnerKind::Spec,
        },
        title: issue.title.clone(),
        active: issue.state == IssueState::Open,
        url: format!("https://github.com/akiojin/gwt/issues/{}", issue.number.0),
        match_basis,
        selectable: issue.state == IssueState::Open,
    }
}

fn combined_corpus_generation(mut fields: Vec<String>) -> String {
    fields.sort();
    let mut digest = Sha256::new();
    for value in
        std::iter::once("gwt.owner-corpus.combined.v1").chain(fields.iter().map(String::as_str))
    {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("gen:v1:{}", hex::encode(digest.finalize()))
}

fn owner_failure_from_api(error: &GitHubApiError) -> OwnerResolutionFailure {
    match error {
        GitHubApiError::PartialPage { .. }
        | GitHubApiError::NotFound(_)
        | GitHubApiError::CommentNotFound(_) => OwnerResolutionFailure {
            reason: BlockedReason::Search,
            failure_subcode: Some(FailureSubcode::PartialPage),
            remediation: "RETRY_OWNER_SEARCH",
        },
        GitHubApiError::Unauthorized | GitHubApiError::PermissionDenied { .. } => {
            OwnerResolutionFailure {
                reason: BlockedReason::Auth,
                failure_subcode: None,
                remediation: "AUTHENTICATE_GITHUB",
            }
        }
        GitHubApiError::Timeout { .. } => OwnerResolutionFailure {
            reason: BlockedReason::Timeout,
            failure_subcode: None,
            remediation: "RETRY_WITHIN_BUDGET",
        },
        GitHubApiError::RateLimited { .. } => OwnerResolutionFailure {
            reason: BlockedReason::RateLimit,
            failure_subcode: None,
            remediation: "RETRY_AFTER_RATE_LIMIT",
        },
        GitHubApiError::Network(_) => OwnerResolutionFailure {
            reason: BlockedReason::Network,
            failure_subcode: None,
            remediation: "RETRY_NETWORK",
        },
        GitHubApiError::Parse { .. }
        | GitHubApiError::Unexpected(_)
        | GitHubApiError::BodyTooLarge => OwnerResolutionFailure {
            reason: BlockedReason::Parse,
            failure_subcode: None,
            remediation: "REFRESH_OWNER_CORPUS",
        },
        GitHubApiError::TestOverrideRejected { .. } | GitHubApiError::RepositoryMismatch { .. } => {
            OwnerResolutionFailure {
                reason: BlockedReason::Routing,
                failure_subcode: None,
                remediation: "REMOVE_UNSAFE_OVERRIDE",
            }
        }
    }
}

pub(super) fn owner_resolution_failure_from_error(
    error: &gwt_github::SpecOpsError,
) -> OwnerResolutionFailure {
    match error {
        gwt_github::SpecOpsError::Api(error) => owner_failure_from_api(error),
        gwt_github::SpecOpsError::Cache(_)
        | gwt_github::SpecOpsError::Parse(_)
        | gwt_github::SpecOpsError::SectionNotFound(_) => OwnerResolutionFailure {
            reason: BlockedReason::Store,
            failure_subcode: None,
            remediation: "RELOAD_CANDIDATE_STORE",
        },
    }
}

fn settle_unhandled_resolution_failure(
    repo_root: &Path,
    token: &ResolutionAttemptToken,
    failure: OwnerResolutionFailure,
) -> Result<bool, gwt_github::SpecOpsError> {
    super::improvement_store::update(repo_root, |store| {
        let candidate = store
            .candidates
            .iter_mut()
            .find(|candidate| candidate.id == token.candidate_id)
            .ok_or_else(|| owner_projection_error("candidate not found"))?;
        if candidate.state != CandidateState::OwnerResolving
            || candidate
                .attempt
                .as_ref()
                .is_none_or(|attempt| attempt.attempt_id != token.attempt_id)
        {
            return Ok(false);
        }
        let now = Utc::now();
        let remote_outcome_unknown = candidate.attempt.as_ref().is_some_and(|attempt| {
            attempt.remote_phase == AttemptRemotePhase::Submitted || attempt.remote_mutation_seen
        });
        candidate.resolver_snapshot = None;
        candidate.updated_at = now.to_rfc3339();
        if remote_outcome_unknown {
            let attempt = candidate
                .attempt
                .as_mut()
                .expect("attempt was validated before settlement");
            attempt.remote_phase = AttemptRemotePhase::Submitted;
            attempt.expires_at = now;
            candidate.blocked_reason = None;
            candidate.failure_subcode = None;
            candidate.retry = Some(RetryMetadata {
                retryable: true,
                remediation: "REFRESH_OWNER_CORPUS".to_string(),
                failed_at: now.to_rfc3339(),
            });
            transition_candidate(candidate, CandidateState::RemoteOutcomeUnknown)?;
        } else {
            candidate.blocked_reason = Some(failure.reason);
            candidate.failure_subcode = failure.failure_subcode;
            candidate.retry = Some(RetryMetadata {
                retryable: true,
                remediation: failure.remediation.to_string(),
                failed_at: now.to_rfc3339(),
            });
            candidate.attempt = None;
            transition_candidate(candidate, CandidateState::Blocked)?;
        }
        Ok(true)
    })
}

fn block_owner_resolution<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    failure: OwnerResolutionFailure,
    resolver_snapshot: Option<ResolverSnapshot>,
    publish_status: bool,
) -> Result<(), gwt_github::SpecOpsError> {
    if candidate.state == CandidateState::RemoteOutcomeUnknown {
        candidate.retry = Some(RetryMetadata {
            retryable: true,
            remediation: "REFRESH_OWNER_CORPUS".to_string(),
            failed_at: Utc::now().to_rfc3339(),
        });
        candidate.resolver_snapshot = resolver_snapshot;
        candidate.updated_at = Utc::now().to_rfc3339();
        return if publish_status {
            post_remote_outcome_unknown_status(env, candidate, failure)
        } else {
            Ok(())
        };
    }
    if matches!(
        candidate.state,
        CandidateState::Blocked | CandidateState::Recurrent
    ) {
        transition_candidate(candidate, CandidateState::OwnerResolving)?;
    }
    candidate.blocked_reason = Some(failure.reason);
    candidate.failure_subcode = failure.failure_subcode;
    candidate.retry = Some(RetryMetadata {
        retryable: true,
        remediation: failure.remediation.to_string(),
        failed_at: Utc::now().to_rfc3339(),
    });
    candidate.resolver_snapshot = resolver_snapshot;
    candidate.updated_at = Utc::now().to_rfc3339();
    transition_candidate(candidate, CandidateState::Blocked)?;
    if publish_status {
        post_owner_resolution_blocked_status(env, candidate, failure)
    } else {
        Ok(())
    }
}

fn post_remote_outcome_unknown_status<E: CliEnv>(
    env: &mut E,
    candidate: &ImprovementCandidate,
    failure: OwnerResolutionFailure,
) -> Result<(), gwt_github::SpecOpsError> {
    let subcode = failure
        .failure_subcode
        .map(failure_subcode_token)
        .unwrap_or("none");
    post_improvement_board_status(
        env,
        format!(
            "Current state: Improvement Candidate {id} remains remote-outcome-unknown.\n\nReason: authoritative refresh failed with {reason}/{subcode}; the prior mutation may exist.\n\nNext: REFRESH_OWNER_CORPUS before any mutation retry.",
            id = candidate.id,
            reason = blocked_reason_token(failure.reason),
        ),
    )
}

fn post_owner_resolution_blocked_status<E: CliEnv>(
    env: &mut E,
    candidate: &ImprovementCandidate,
    failure: OwnerResolutionFailure,
) -> Result<(), gwt_github::SpecOpsError> {
    let subcode = failure
        .failure_subcode
        .map(failure_subcode_token)
        .unwrap_or("none");
    post_improvement_board_status(
        env,
        format!(
            "Current state: Improvement Candidate {id} owner resolution is blocked.\n\nReason: {reason}/{subcode}.\n\nNext: {remediation}.",
            id = candidate.id,
            reason = blocked_reason_token(failure.reason),
            remediation = failure.remediation,
        ),
    )
}

fn blocked_reason_token(reason: BlockedReason) -> &'static str {
    match reason {
        BlockedReason::Store => "store",
        BlockedReason::Search => "search",
        BlockedReason::Auth => "auth",
        BlockedReason::Privacy => "privacy",
        BlockedReason::Ambiguity => "ambiguity",
        BlockedReason::Routing => "routing",
        BlockedReason::Create => "create",
        BlockedReason::Update => "update",
        BlockedReason::Readback => "readback",
        BlockedReason::LocalCommit => "local-commit",
        BlockedReason::Timeout => "timeout",
        BlockedReason::RateLimit => "rate-limit",
        BlockedReason::Network => "network",
        BlockedReason::Parse => "parse",
        BlockedReason::Reconciliation => "reconciliation",
    }
}

fn failure_subcode_token(subcode: FailureSubcode) -> &'static str {
    match subcode {
        FailureSubcode::EmptyCorpus => "empty-corpus",
        FailureSubcode::PartialPage => "partial-page",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PublicIssuePayload {
    pub(super) summary: String,
    pub(super) title: String,
    pub(super) body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct PublicCommentPayload {
    pub(super) body: String,
}

#[derive(Debug, Clone)]
pub(super) struct PublicMutationContext {
    denied_values: Vec<String>,
    collection_complete: bool,
}

impl Default for PublicMutationContext {
    fn default() -> Self {
        Self {
            denied_values: Vec::new(),
            collection_complete: true,
        }
    }
}

impl PublicMutationContext {
    pub(super) fn for_repo(repo_root: &Path) -> Self {
        let expires_at = Instant::now()
            .checked_add(Duration::from_secs(3))
            .unwrap_or_else(Instant::now);
        Self::for_repo_until(repo_root, expires_at)
    }

    fn for_repo_with_deadline(repo_root: &Path, deadline: &ResolutionDeadline) -> Self {
        Self::for_repo_until(repo_root, deadline.expires_at())
    }

    fn for_repo_until(repo_root: &Path, expires_at: Instant) -> Self {
        let mut denied = BTreeSet::new();
        let mut collection_complete = true;
        add_path(&mut denied, repo_root);
        match std::fs::canonicalize(repo_root) {
            Ok(canonical) => add_path(&mut denied, &canonical),
            Err(_) => collection_complete = false,
        }
        let has_git_metadata = match repo_root.join(".git").try_exists() {
            Ok(exists) => exists,
            Err(_) => {
                collection_complete = false;
                false
            }
        };
        match gwt_git::worktree::main_worktree_root(repo_root) {
            Ok(main_root) => {
                add_path(&mut denied, &main_root);
                match gwt_git::WorktreeManager::new(main_root).list() {
                    Ok(worktrees) => {
                        for worktree in worktrees {
                            add_path(&mut denied, &worktree.path);
                        }
                    }
                    Err(_) => collection_complete = false,
                }
            }
            Err(_) if has_git_metadata => collection_complete = false,
            Err(_) => {}
        }
        match source_repository_slug(repo_root, expires_at) {
            Ok(Some(slug)) => add_denied_value(&mut denied, slug),
            Ok(None) => {}
            Err(()) => collection_complete = false,
        }
        for key in ["HOME", "USERPROFILE", "USER", "USERNAME"] {
            if let Ok(value) = std::env::var(key) {
                add_denied_value(&mut denied, value);
            }
        }
        add_secret_environment_values(&mut denied, std::env::vars());
        let config_path = gwt_core::paths::gwt_config_path();
        let config_exists = match config_path.try_exists() {
            Ok(exists) => exists,
            Err(_) => {
                collection_complete = false;
                false
            }
        };
        if config_exists {
            match gwt_agent::load_custom_agents_from_path(&config_path) {
                Ok(agents) => {
                    for agent in agents {
                        add_secret_environment_values(&mut denied, agent.env);
                    }
                }
                Err(_) => collection_complete = false,
            }
        }
        Self {
            denied_values: denied.into_iter().collect(),
            collection_complete,
        }
    }

    fn with_candidate(&self, candidate: &ImprovementCandidate, trusted_values: &[&str]) -> Self {
        let mut denied = self.denied_values.iter().cloned().collect::<BTreeSet<_>>();
        add_denied_value(&mut denied, candidate.sanitized_summary.clone());
        if let Some(details) = &candidate.sanitized_details {
            add_denied_value(&mut denied, details.clone());
        }
        if let Some(evidence_digest) = &candidate.evidence_digest {
            if !trusted_values.contains(&evidence_digest.as_str()) {
                add_denied_value(&mut denied, evidence_digest.clone());
            }
        }
        add_denied_value(&mut denied, candidate.dedupe_key.clone());
        for evidence in &candidate.local_evidence {
            if let Some(path) = &evidence.path {
                add_denied_value(&mut denied, path.clone());
            }
        }
        for occurrence in &candidate.distinct_occurrences {
            match occurrence.replay_proof.as_ref() {
                Some(OccurrenceReplayProof::InterpretiveSession {
                    source_scope_nonce,
                    session_id,
                }) => {
                    add_denied_value(&mut denied, source_scope_nonce.clone());
                    add_denied_value(&mut denied, session_id.clone());
                }
                Some(OccurrenceReplayProof::RegisteredEvent {
                    source_scope_nonce,
                    source_event_id,
                }) => {
                    add_denied_value(&mut denied, source_scope_nonce.clone());
                    add_denied_value(&mut denied, source_event_id.clone());
                }
                None => {}
            }
        }
        Self {
            denied_values: denied.into_iter().collect(),
            collection_complete: self.collection_complete,
        }
    }

    #[cfg(test)]
    fn from_denied_values_for_test<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut denied = BTreeSet::new();
        for value in values {
            add_denied_value(&mut denied, value.into());
        }
        Self {
            denied_values: denied.into_iter().collect(),
            collection_complete: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PrivacyViolationKind {
    ContextIncomplete,
    DynamicValue,
    UnixPath,
    WindowsPath,
    Secret,
    Authorization,
    UrlCredential,
    UrlQuerySecret,
    PrivateKey,
    Email,
    LogExcerpt,
    CodeExcerpt,
    SizeLimit,
    InvalidTemplateField,
}

impl PrivacyViolationKind {
    const fn code(self) -> &'static str {
        match self {
            Self::ContextIncomplete => "CONTEXT_INCOMPLETE",
            Self::DynamicValue => "DYNAMIC_VALUE",
            Self::UnixPath => "UNIX_PATH",
            Self::WindowsPath => "WINDOWS_PATH",
            Self::Secret => "SECRET",
            Self::Authorization => "AUTHORIZATION",
            Self::UrlCredential => "URL_CREDENTIAL",
            Self::UrlQuerySecret => "URL_QUERY_SECRET",
            Self::PrivateKey => "PRIVATE_KEY",
            Self::Email => "EMAIL",
            Self::LogExcerpt => "LOG_EXCERPT",
            Self::CodeExcerpt => "CODE_EXCERPT",
            Self::SizeLimit => "SIZE_LIMIT",
            Self::InvalidTemplateField => "INVALID_TEMPLATE_FIELD",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PrivacyViolation {
    kind: PrivacyViolationKind,
}

impl PrivacyViolation {
    const fn new(kind: PrivacyViolationKind) -> Self {
        Self { kind }
    }

    #[cfg(test)]
    pub(super) const fn kind(&self) -> PrivacyViolationKind {
        self.kind
    }
}

impl fmt::Display for PrivacyViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "public mutation rejected by privacy gate: {}",
            self.kind.code()
        )
    }
}

impl std::error::Error for PrivacyViolation {}

pub(super) fn render_public_issue_payload(
    candidate: &ImprovementCandidate,
    context: &PublicMutationContext,
) -> Result<PublicIssuePayload, PrivacyViolation> {
    validate_candidate_template_fields(candidate)?;
    let (summary, problem, expected, observed, public_identity) = match &candidate.typed_evidence {
        Some(evidence) => {
            let (summary, problem, expected, observed) =
                typed_template_fields(candidate, evidence)?;
            (
                summary,
                problem,
                expected,
                observed,
                Some(typed_public_identity(candidate, evidence)?),
            )
        }
        None => (
            "Self-improvement contract evidence required".to_string(),
            "A legacy self-improvement candidate does not contain typed contract evidence."
                .to_string(),
            "Typed evidence is required before unattended owner resolution.".to_string(),
            "No free-form candidate text is rendered into a public mutation.".to_string(),
            None,
        ),
    };
    let title = format!("fix(gwt): {summary}");
    let identity = public_identity
        .as_ref()
        .map(|identity| {
            format!(
                "- Public evidence digest: sha256:{}\n\n{}\n",
                identity.evidence_digest,
                fingerprint_marker(&identity.fingerprint)
            )
        })
        .unwrap_or_else(|| {
            "- Typed public identity: unavailable for legacy evidence\n".to_string()
        });
    let body = format!(
        "## Problem\n\n{problem}\n\n## Expected behavior\n\n{expected}\n\n## Observed evidence\n\n{observed}\n\n{identity}\n## Impact\n\nA gwt-owned contract failure can recur until it has one verified upstream owner.\n\n## Suggested verification\n\n- Reproduce the typed contract outcome.\n- Add a regression test that fails before the fix.\n- Verify the corrected outcome without private source data.\n\n## Source candidate\n\n- Candidate ID: {id}\n- Target artifact: {target}\n- Classification: {classification}\n- Confidence: {confidence}\n\n## Privacy\n\n- This payload is generated from contract-owned typed fields.\n- Free-form evidence, repository identity, paths, credentials, logs, and code remain local-only.\n",
        id = candidate.id,
        target = candidate.target_artifact,
        classification = candidate.classification,
        confidence = candidate.confidence,
    );
    let payload = PublicIssuePayload {
        summary,
        title,
        body,
    };
    let trusted_values = public_identity
        .as_ref()
        .map(|identity| {
            vec![
                identity.evidence_digest.as_str(),
                identity.fingerprint.as_str(),
            ]
        })
        .unwrap_or_default();
    validate_public_payload(
        &payload,
        &context.with_candidate(candidate, &trusted_values),
    )?;
    Ok(payload)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TypedPublicIdentity {
    fingerprint: String,
    evidence_digest: String,
}

fn typed_public_identity(
    candidate: &ImprovementCandidate,
    evidence: &TypedFailureEvidence,
) -> Result<TypedPublicIdentity, PrivacyViolation> {
    let fingerprint = improvement_fingerprint(evidence);
    if candidate.fingerprint.as_deref() != Some(fingerprint.as_str()) {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    Ok(TypedPublicIdentity {
        fingerprint,
        evidence_digest: typed_evidence_digest(evidence),
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn render_occurrence_comment_payload(
    candidate: &ImprovementCandidate,
    occurrence_key: &str,
    context: &PublicMutationContext,
) -> Result<PublicCommentPayload, PrivacyViolation> {
    validate_candidate_template_fields(candidate)?;
    if !occurrence_key_re().is_match(occurrence_key) {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    let evidence = candidate
        .typed_evidence
        .as_ref()
        .ok_or_else(|| PrivacyViolation::new(PrivacyViolationKind::InvalidTemplateField))?;
    let identity = typed_public_identity(candidate, evidence)?;
    let body = format!(
        "gwt recorded one typed self-improvement occurrence.\n\n- Candidate ID: {id}\n- Public evidence digest: sha256:{digest}\n\n{occurrence_marker}\n{fingerprint_marker}\n",
        id = candidate.id,
        digest = identity.evidence_digest,
        occurrence_marker = occurrence_marker(occurrence_key),
        fingerprint_marker = fingerprint_marker(&identity.fingerprint),
    );
    validate_public_comment(
        &body,
        &context.with_candidate(
            candidate,
            &[&identity.evidence_digest, &identity.fingerprint],
        ),
    )?;
    Ok(PublicCommentPayload { body })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn render_reconciliation_comment_payload(
    candidate: &ImprovementCandidate,
    canonical_number: u64,
    duplicate_number: u64,
    context: &PublicMutationContext,
) -> Result<PublicCommentPayload, PrivacyViolation> {
    validate_candidate_template_fields(candidate)?;
    if canonical_number == 0 || duplicate_number == 0 || canonical_number == duplicate_number {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    let evidence = candidate
        .typed_evidence
        .as_ref()
        .ok_or_else(|| PrivacyViolation::new(PrivacyViolationKind::InvalidTemplateField))?;
    let identity = typed_public_identity(candidate, evidence)?;
    let body = format!(
        "This Issue duplicates the same typed self-improvement owner.\n\n- Canonical owner: #{canonical_number}\n- Duplicate owner: #{duplicate_number}\n- Public evidence digest: sha256:{digest}\n\n<!-- gwt:improvement-reconciliation:v1 canonical:{canonical_number} duplicate:{duplicate_number} -->\n{fingerprint_marker}\n",
        digest = identity.evidence_digest,
        fingerprint_marker = fingerprint_marker(&identity.fingerprint),
    );
    validate_public_comment(
        &body,
        &context.with_candidate(
            candidate,
            &[&identity.evidence_digest, &identity.fingerprint],
        ),
    )?;
    Ok(PublicCommentPayload { body })
}

#[cfg_attr(not(test), allow(dead_code))]
fn validate_public_comment(
    body: &str,
    context: &PublicMutationContext,
) -> Result<(), PrivacyViolation> {
    validate_public_payload(
        &PublicIssuePayload {
            summary: "Typed self-improvement occurrence".to_string(),
            title: "Typed self-improvement occurrence".to_string(),
            body: body.to_string(),
        },
        context,
    )
}

fn fingerprint_marker(fingerprint: &str) -> String {
    format!("<!-- gwt:improvement-fingerprint:v1 {fingerprint} -->")
}

#[cfg_attr(not(test), allow(dead_code))]
fn occurrence_marker(occurrence_key: &str) -> String {
    format!("<!-- gwt:improvement-occurrence:v1 {occurrence_key} -->")
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn exact_fingerprint_markers(body: &str) -> Vec<String> {
    exact_marker_values(body, fingerprint_marker_re())
}

#[cfg_attr(not(test), allow(dead_code))]
fn exact_occurrence_markers(body: &str) -> Vec<String> {
    exact_marker_values(body, occurrence_marker_re())
}

fn exact_reconciliation_markers(body: &str) -> Vec<String> {
    exact_marker_values(body, reconciliation_marker_re())
}

fn exact_marker_values(body: &str, marker_re: &Regex) -> Vec<String> {
    let mut code_fence: Option<(u8, usize)> = None;
    body.lines()
        .filter_map(|line| {
            if let Some((delimiter, length, trailing_blank)) = markdown_fence_delimiter(line) {
                match code_fence {
                    Some((open_delimiter, open_length))
                        if delimiter == open_delimiter
                            && length >= open_length
                            && trailing_blank =>
                    {
                        code_fence = None;
                        return None;
                    }
                    None => {
                        code_fence = Some((delimiter, length));
                        return None;
                    }
                    _ => {}
                }
            }
            if code_fence.is_some() {
                return None;
            }
            marker_re
                .captures(line)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().to_string())
        })
        .collect()
}

fn comment_matches_occurrence(
    comment: &RepositoryComment,
    occurrence_key: &str,
    fingerprint: &str,
) -> bool {
    exact_occurrence_markers(&comment.body)
        .iter()
        .any(|marker| marker == occurrence_key)
        && exact_fingerprint_markers(&comment.body)
            .iter()
            .any(|marker| marker == fingerprint)
}

fn comment_matches_reconciliation(
    comment: &RepositoryComment,
    canonical_owner_number: u64,
    duplicate_owner_number: u64,
    fingerprint: &str,
) -> bool {
    let expected = format!("canonical:{canonical_owner_number} duplicate:{duplicate_owner_number}");
    let reconciliation_markers = exact_reconciliation_markers(&comment.body);
    let fingerprint_markers = exact_fingerprint_markers(&comment.body);
    reconciliation_markers.len() == 1
        && reconciliation_markers[0] == expected
        && fingerprint_markers.len() == 1
        && fingerprint_markers[0] == fingerprint
}

fn markdown_fence_delimiter(line: &str) -> Option<(u8, usize, bool)> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let content = &line[indent..];
    let delimiter = content.bytes().next()?;
    if !matches!(delimiter, b'`' | b'~') {
        return None;
    }
    let length = content
        .bytes()
        .take_while(|byte| *byte == delimiter)
        .count();
    (length >= 3).then(|| {
        let trailing_blank = content[length..].trim().is_empty();
        (delimiter, length, trailing_blank)
    })
}

fn typed_template_fields(
    candidate: &ImprovementCandidate,
    evidence: &TypedFailureEvidence,
) -> Result<(String, String, String, String), PrivacyViolation> {
    for value in [
        evidence.subsystem.as_str(),
        evidence.contract_id.as_str(),
        evidence.failure_code.as_str(),
        evidence.target_artifact.as_str(),
        evidence.expected_outcome.as_str(),
        evidence.observed_outcome.as_str(),
    ] {
        if !machine_token_re().is_match(value) {
            return Err(PrivacyViolation::new(
                PrivacyViolationKind::InvalidTemplateField,
            ));
        }
    }
    if evidence.target_artifact != candidate.target_artifact {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    let summary = truncate_chars(
        &format!("{}: {}", candidate.target_artifact, evidence.failure_code),
        MAX_PUBLIC_SUMMARY_CHARS,
    );
    Ok((
        summary,
        format!(
            "The gwt-owned `{}` contract `{}` reported `{}`.",
            evidence.subsystem, evidence.contract_id, evidence.failure_code
        ),
        format!("`{}`", evidence.expected_outcome),
        format!("`{}`", evidence.observed_outcome),
    ))
}

fn validate_candidate_template_fields(
    candidate: &ImprovementCandidate,
) -> Result<(), PrivacyViolation> {
    let target_valid = [
        "skill",
        "AGENTS",
        "hook",
        "launch",
        "index",
        "verification",
        "coordination",
        "issue-spec-workflow",
        "unknown",
    ]
    .contains(&candidate.target_artifact.as_str());
    let classification_valid = ["gwt-caused", "ambiguous", "target-project", "external"]
        .contains(&candidate.classification.as_str());
    let confidence_valid = ["low", "medium", "high"].contains(&candidate.confidence.as_str());
    let id_valid = candidate.id.len() <= 96
        && candidate.id.starts_with("impr-")
        && candidate
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !target_valid || !classification_valid || !confidence_valid || !id_valid {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::InvalidTemplateField,
        ));
    }
    Ok(())
}

pub(super) fn validate_public_payload(
    payload: &PublicIssuePayload,
    context: &PublicMutationContext,
) -> Result<(), PrivacyViolation> {
    if !context.collection_complete {
        return Err(PrivacyViolation::new(
            PrivacyViolationKind::ContextIncomplete,
        ));
    }
    if payload.summary.chars().count() > MAX_PUBLIC_SUMMARY_CHARS
        || payload.title.chars().count() > MAX_PUBLIC_TITLE_CHARS
        || payload.title.contains(['\r', '\n'])
        || payload.body.len() > MAX_PUBLIC_BODY_BYTES
    {
        return Err(PrivacyViolation::new(PrivacyViolationKind::SizeLimit));
    }
    let combined = format!("{}\n{}\n{}", payload.summary, payload.title, payload.body);
    let lower = combined.to_ascii_lowercase();
    if context.denied_values.iter().any(|value| {
        combined.contains(value) || lower.contains(value.to_ascii_lowercase().as_str())
    }) {
        return Err(PrivacyViolation::new(PrivacyViolationKind::DynamicValue));
    }
    for (regex, kind) in [
        (authorization_re(), PrivacyViolationKind::Authorization),
        (url_credential_re(), PrivacyViolationKind::UrlCredential),
        (url_query_secret_re(), PrivacyViolationKind::UrlQuerySecret),
        (private_key_re(), PrivacyViolationKind::PrivateKey),
        (secret_re(), PrivacyViolationKind::Secret),
        (email_re(), PrivacyViolationKind::Email),
        (windows_path_re(), PrivacyViolationKind::WindowsPath),
        (unix_path_re(), PrivacyViolationKind::UnixPath),
        (log_excerpt_re(), PrivacyViolationKind::LogExcerpt),
        (code_excerpt_re(), PrivacyViolationKind::CodeExcerpt),
    ] {
        if regex.is_match(&combined) {
            return Err(PrivacyViolation::new(kind));
        }
    }
    Ok(())
}

fn add_path(denied: &mut BTreeSet<String>, path: &Path) {
    add_denied_value(denied, path.to_string_lossy().into_owned());
}

fn add_denied_value(denied: &mut BTreeSet<String>, value: String) {
    let value = value.trim();
    if !value.is_empty() && !value.contains("[redacted-") && value != "***redacted***" {
        denied.insert(value.to_string());
    }
}

fn source_repository_slug(repo_root: &Path, expires_at: Instant) -> Result<Option<String>, ()> {
    let repo_root = repo_root.to_str().ok_or(())?;
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking_with_deadline(
        &hub,
        gwt_core::process_console::ProcessKind::Git,
        "git",
        &["-C", repo_root, "config", "--get", "remote.origin.url"],
        gwt_core::process_console::SpawnOptions::new("git remote origin").forward_output(false),
        expires_at,
    )
    .map_err(|_| ())?;
    if !output.success() {
        return if output.exit_code == Some(1) {
            Ok(None)
        } else {
            Err(())
        };
    }
    let normalized = gwt_core::repo_hash::normalize_origin_url(output.stdout.trim());
    Ok(normalized
        .split_once('/')
        .map(|(_, slug)| slug.to_string())
        .filter(|slug| slug.contains('/')))
}

fn is_secret_environment_key(key: &str) -> bool {
    let normalized = key.to_ascii_uppercase();
    gwt_agent::is_secret_env_key(key)
        || normalized.contains("SECRET")
        || normalized.contains("TOKEN")
        || normalized.contains("PASSWORD")
        || normalized.contains("PASSWD")
        || normalized.contains("AUTH")
        || normalized.ends_with("_KEY")
        || normalized.contains("API_KEY")
}

fn add_secret_environment_values<I>(denied: &mut BTreeSet<String>, values: I)
where
    I: IntoIterator<Item = (String, String)>,
{
    for (key, value) in values {
        if is_secret_environment_key(&key) {
            add_denied_value(denied, value);
        }
    }
}

fn truncate_chars(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    let mut output = input
        .chars()
        .take(max.saturating_sub(3))
        .collect::<String>();
    output.push_str("...");
    output
}

fn machine_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$").expect("machine token regex")
    })
}

#[cfg_attr(not(test), allow(dead_code))]
fn fingerprint_marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^<!-- gwt:improvement-fingerprint:v1 (v2:[0-9a-f]{64}) -->$")
            .expect("fingerprint marker regex")
    })
}

fn fingerprint_value_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^v2:[0-9a-f]{64}$").expect("fingerprint value regex"))
}

#[cfg_attr(not(test), allow(dead_code))]
fn occurrence_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^occ:v1:[0-9a-f]{64}$").expect("occurrence key regex"))
}

fn occurrence_marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^<!-- gwt:improvement-occurrence:v1 (occ:v1:[0-9a-f]{64}) -->$")
            .expect("occurrence marker regex")
    })
}

fn reconciliation_marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^<!-- gwt:improvement-reconciliation:v1 (canonical:[1-9][0-9]* duplicate:[1-9][0-9]*) -->$",
        )
        .expect("reconciliation marker regex")
    })
}

fn authorization_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)authorization\s*:").expect("authorization regex"))
}

fn url_credential_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)https?://[^\s/:@]+:[^\s/@]+@").expect("URL credential regex")
    })
}

fn url_query_secret_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)[?&][^\s=&]*(?:token|secret|password|passwd|key|auth)[^\s=&]*=")
            .expect("URL query secret regex")
    })
}

fn private_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----").expect("private key regex")
    })
}

fn secret_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:\b(?:gh[pousr]|github_pat)_[A-Za-z0-9_]{16,}\b|\b(?:api[_-]?key|token|secret|password|passwd|auth)\s*[=:]\s*[^\s]+)",
        )
        .expect("secret regex")
    })
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
            .expect("email regex")
    })
}

fn windows_path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:\b[A-Z]:\\|\\\\[A-Za-z0-9._-]+\\)[^\r\n]*").expect("Windows path regex")
    })
}

fn unix_path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?:^|[\s(\"'])/(?:[^/\s]+/)+[^/\s]+"#).expect("Unix path regex")
    })
}

fn log_excerpt_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:^|\s)(?:TRACE|DEBUG|INFO|WARN|ERROR)(?:\s|:)|stack backtrace:|thread '[^']+' panicked")
            .expect("log excerpt regex")
    })
}

fn code_excerpt_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"```").expect("code excerpt regex"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::*;
    use crate::cli::{
        improvement::{
            occurrence_evidence_digest, opaque_occurrence_key, CandidateState, DistinctOccurrence,
            DurableOwnerSnapshot, ImprovementCandidate, ImprovementEligibility, OccurrenceOrigin,
            OccurrenceReplayProof, OwnerKind, OwnerMatchBasis, TypedRecurrenceEvidence,
        },
        run_collect, BoardCommand, CliCommand, TestEnv,
    };
    use gwt_github::client::{
        fake::{FakeIssueClient, OwnerRepositoryFaultTiming, OwnerRepositoryOperation},
        ApiError as GitHubApiError, CommentId, CommitComparison, CommitComparisonStatus,
        IssueNumber, IssueState, MergedPullRequest, RepositoryActorType,
        RepositoryAuthorAssociation, RepositoryComment, RepositoryIdentity, RepositoryIssue,
        RepositoryIssueKind, RepositoryRelease, ResolutionDeadline, UpdatedAt,
    };

    fn commit_binding_with_complete_comment_audit(
        source_repo_root: &Path,
        binding: &ReadbackVerifiedOwnerBinding,
        comment_ids: &[u64],
    ) -> Result<crate::cli::improvement_store::OwnerProjectionCommitOutcome, gwt_github::SpecOpsError>
    {
        let mut commit = prepare_owner_projection_commit(source_repo_root, binding)?;
        commit.occurrence.comment_audit = crate::cli::improvement_store::StoredCommentAudit {
            completeness: crate::cli::improvement_store::StoredCommentAuditCompleteness::Complete,
            physical_comments: comment_ids
                .iter()
                .copied()
                .map(
                    |comment_id| crate::cli::improvement_store::StoredCommentRef {
                        owner_number: binding.owner.number,
                        comment_id,
                    },
                )
                .collect(),
        };
        crate::cli::improvement_store::commit_owner_projection(commit)
    }

    fn candidate(typed: bool) -> ImprovementCandidate {
        let fingerprint = "v2:4bea839977a5aeedbf562acaeeb547012b0447f3335279830405fafb37726532";
        let source_scope_nonce = "0".repeat(64);
        let source_event_id = "event-1";
        serde_json::from_value(json!({
            "schema_version": 3,
            "id": "impr-public-template",
            "created_at": "2026-07-14T00:00:00Z",
            "updated_at": "2026-07-14T00:00:00Z",
            "source": "agent-failure",
            "target_artifact": "coordination",
            "classification": "gwt-caused",
            "confidence": "high",
            "state": "owner-resolving",
            "dedupe_key": "private-repo:customer-8675309",
            "occurrences": 1,
            "fingerprint": fingerprint,
            "eligibility": "deterministic",
            "typed_evidence": typed.then(|| json!({
                "subsystem": "coordination",
                "contract_id": "coordination.board-status",
                "contract_schema_revision": 1,
                "failure_code": "STATUS_NOT_POSTED",
                "target_artifact": "coordination",
                "expected_outcome": "BOARD_STATUS_POSTED",
                "observed_outcome": "BOARD_STATUS_MISSING"
            })),
            "distinct_occurrences": if typed { vec![json!({
                "opaque_key": opaque_occurrence_key(
                    &source_scope_nonce,
                    fingerprint,
                    "test.coordination-gate.v1",
                    source_event_id,
                ),
                "evidence_digest": "3f649bd386b953b42442e8cefcbd1449d657f49a972f11d72f810bcda167756a",
                "captured_at": "2026-07-14T00:00:00Z",
                "origin": "deterministic",
                "qualifies_unattended": true,
                "producer_id": "test.coordination-gate.v1",
                "producer_registry_revision": 1,
                "routing_basis_revision": 1,
                "replay_proof": {
                    "kind": "registered-event",
                    "source_scope_nonce": source_scope_nonce,
                    "source_event_id": source_event_id
                }
            })] } else { Vec::new() },
            "sanitized_summary": "Customer customer-8675309 failed at /Users/alice/private-repo",
            "sanitized_details": "Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz",
            "evidence_digest": "alice@example.com",
            "local_evidence": [{
                "kind": "transcript",
                "path": "C:\\Users\\alice\\private-repo\\trace.log"
            }],
            "linked_issue": null,
            "dismissed_reason": null
        }))
        .expect("candidate")
    }

    fn payload_with_body(body: &str) -> PublicIssuePayload {
        PublicIssuePayload {
            summary: "Safe contract failure".to_string(),
            title: "fix(gwt): Safe contract failure".to_string(),
            body: body.to_string(),
        }
    }

    fn resolution_deadline() -> ResolutionDeadline {
        ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5))
    }

    const MERGE_COMMIT_SHA: &str = "1111111111111111111111111111111111111111";
    const RELEASE_COMMIT_SHA: &str = "2222222222222222222222222222222222222222";
    const BUILD_COMMIT_SHA: &str = "3333333333333333333333333333333333333333";
    const OTHER_COMMIT_SHA: &str = "4444444444444444444444444444444444444444";
    const CONFLICTING_COMMIT_SHA: &str = "5555555555555555555555555555555555555555";

    fn implementation_gap_registry(routing_basis_revision: u64) -> ContractRoutingRegistry {
        ContractRoutingRegistry::new(vec![ContractRouteMapping {
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            routing_basis_revision,
            disposition: ContractRouteDisposition::ImplementationGap,
        }])
    }

    fn owner_issue(number: u64, state: IssueState, body: impl Into<String>) -> RepositoryIssue {
        RepositoryIssue {
            repository: RepositoryIdentity::gwt_upstream(),
            number: IssueNumber(number),
            title: format!("Owner {number}"),
            body: body.into(),
            labels: Vec::new(),
            state,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new(format!("u{number}")),
        }
    }

    fn historical_resolution_comment(
        merged_pr_number: u64,
        merge_commit_sha: &str,
        verified_at: &str,
        first_fixed_release: &str,
    ) -> String {
        format!(
            "<!-- gwt-improvement-resolution:v1 {{\"merged_pr_number\":{merged_pr_number},\"merge_commit_sha\":\"{merge_commit_sha}\",\"verified_at\":\"{verified_at}\",\"first_fixed_release\":\"{first_fixed_release}\"}} -->"
        )
    }

    fn repository_comment(
        id: u64,
        body: impl Into<String>,
        updated_at: impl Into<String>,
    ) -> RepositoryComment {
        RepositoryComment {
            id: CommentId(id),
            body: body.into(),
            updated_at: UpdatedAt::new(updated_at.into()),
            author_login: Some("test-owner".to_string()),
            author_type: Some(RepositoryActorType::User),
            author_association: Some(RepositoryAuthorAssociation::Owner),
        }
    }

    fn historical_regression_fixture(
        source: &Path,
        candidate_id: &str,
        recurrence: TypedRecurrenceEvidence,
        comments: Vec<RepositoryComment>,
    ) -> (String, TestEnv) {
        let nonce = "0".repeat(64);
        let (fingerprint, _) =
            store_projection_candidate(source, &nonce, candidate_id, "historical-regression-event");
        crate::cli::improvement_store::update(source, |store| {
            let recurrent = &mut store.candidates[0];
            recurrent.distinct_occurrences[0].evidence_digest = occurrence_evidence_digest(
                recurrent.typed_evidence.as_ref().expect("typed evidence"),
                Some(&recurrence),
            );
            recurrent.distinct_occurrences[0].captured_at = "2026-07-16T00:00:00Z".to_string();
            recurrent.distinct_occurrences[0].recurrence = Some(recurrence);
            recurrent.state = CandidateState::Recurrent;
            recurrent.owner = Some(DurableOwnerSnapshot {
                number: 45,
                kind: OwnerKind::Issue,
                title: "Owner 45".to_string(),
                active: true,
                url: "https://github.com/akiojin/gwt/issues/45".to_string(),
                fingerprint: fingerprint.clone(),
                readback_verified_at: "2026-07-10T00:00:00Z".to_string(),
            });
            Ok(())
        })
        .expect("store recurrent candidate");

        let repository = RepositoryIdentity::gwt_upstream();
        let mut env = TestEnv::new(source.join("cache"));
        env.repo_path = source.to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Closed,
            fingerprint_marker(&fingerprint),
        ));
        env.owner_client
            .seed_repository_comments(&repository, IssueNumber(45), comments);
        env.owner_client.seed_merged_pull_request(
            &repository,
            MergedPullRequest {
                number: IssueNumber(900),
                merge_commit_sha: MERGE_COMMIT_SHA.to_string(),
                merged_at: "2026-07-01T00:00:00Z".to_string(),
            },
        );
        env.owner_client.seed_repository_release(
            &repository,
            RepositoryRelease {
                tag_name: "v9.65.0".to_string(),
                target_commitish: "develop".to_string(),
                published_at: "2026-07-05T00:00:00Z".to_string(),
            },
        );
        env.owner_client.seed_commit_comparison(
            &repository,
            CommitComparison {
                base: MERGE_COMMIT_SHA.to_string(),
                head: "refs/tags/v9.65.0".to_string(),
                base_commit_sha: MERGE_COMMIT_SHA.to_string(),
                merge_base_commit_sha: MERGE_COMMIT_SHA.to_string(),
                head_commit_sha: RELEASE_COMMIT_SHA.to_string(),
                status: CommitComparisonStatus::Ahead,
                ahead_by: 4,
                behind_by: 0,
            },
        );
        (fingerprint, env)
    }

    fn standard_historical_resolution_comment() -> RepositoryComment {
        repository_comment(
            501,
            historical_resolution_comment(900, MERGE_COMMIT_SHA, "2026-07-10T00:00:00Z", "v9.65.0"),
            "2026-07-10T00:00:00Z",
        )
    }

    fn version_recurrence(version: Option<&str>, observed_at: &str) -> TypedRecurrenceEvidence {
        TypedRecurrenceEvidence {
            installed_version: version.map(str::to_string),
            build_commit: None,
            observed_at: observed_at.to_string(),
        }
    }

    struct StaticSemanticAdvisor(Result<Vec<IssueNumber>, ()>);

    impl SemanticOwnerAdvisor for StaticSemanticAdvisor {
        fn owner_numbers(
            &self,
            _candidate: &ImprovementCandidate,
            _deadline: &ResolutionDeadline,
        ) -> Result<Vec<IssueNumber>, ()> {
            self.0.clone()
        }
    }

    struct SlowSemanticAdvisor(Duration);

    impl SemanticOwnerAdvisor for SlowSemanticAdvisor {
        fn owner_numbers(
            &self,
            _candidate: &ImprovementCandidate,
            _deadline: &ResolutionDeadline,
        ) -> Result<Vec<IssueNumber>, ()> {
            std::thread::sleep(self.0);
            Ok(Vec::new())
        }
    }

    struct CorpusMutatingAdvisor<'a> {
        client: &'a FakeIssueClient,
        issue: RepositoryIssue,
    }

    impl SemanticOwnerAdvisor for CorpusMutatingAdvisor<'_> {
        fn owner_numbers(
            &self,
            _candidate: &ImprovementCandidate,
            _deadline: &ResolutionDeadline,
        ) -> Result<Vec<IssueNumber>, ()> {
            self.client.seed_repository_issue(self.issue.clone());
            Ok(Vec::new())
        }
    }

    #[derive(Default)]
    struct DeadlineRecordingAdvisor {
        expires_at: Mutex<Option<Instant>>,
    }

    impl SemanticOwnerAdvisor for DeadlineRecordingAdvisor {
        fn owner_numbers(
            &self,
            _candidate: &ImprovementCandidate,
            deadline: &ResolutionDeadline,
        ) -> Result<Vec<IssueNumber>, ()> {
            *self.expires_at.lock().expect("deadline recording lock") = Some(deadline.expires_at());
            Ok(Vec::new())
        }
    }

    fn board_bodies(env: &mut TestEnv) -> Vec<String> {
        let (_, output) = run_collect(
            env,
            CliCommand::Board(BoardCommand::Show {
                json: true,
                workspace: None,
                all: true,
            }),
        )
        .expect("board show");
        serde_json::from_str::<serde_json::Value>(&output).expect("board json")["board"]["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| entry["body"].as_str().expect("body").to_string())
            .collect()
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn repository_context_collects_slug_roots_worktrees_and_secret_key_shapes() {
        let root = tempfile::tempdir().expect("root");
        let repository = root.path().join("private-repo");
        let worktree = root.path().join("private-repo-worktree");
        std::fs::create_dir_all(&repository).expect("repository");
        run_git(&repository, &["init"]);
        run_git(
            &repository,
            &["config", "user.email", "test@example.invalid"],
        );
        run_git(&repository, &["config", "user.name", "Test"]);
        run_git(
            &repository,
            &[
                "remote",
                "add",
                "origin",
                "https://gitlab.example/acme/private-repo.git",
            ],
        );
        std::fs::write(repository.join("README.md"), "fixture\n").expect("README");
        run_git(&repository, &["add", "README.md"]);
        run_git(&repository, &["commit", "-m", "test: fixture"]);
        run_git(
            &repository,
            &[
                "worktree",
                "add",
                "--detach",
                worktree.to_str().expect("worktree path"),
                "HEAD",
            ],
        );

        let context = PublicMutationContext::for_repo(&worktree);
        assert!(context
            .denied_values
            .contains(&"acme/private-repo".to_string()));
        assert!(context
            .denied_values
            .iter()
            .any(|value| value.contains("private-repo-worktree")));
        assert!(context
            .denied_values
            .iter()
            .any(|value| value.contains("private-repo")));
        for key in [
            "GITHUB_TOKEN",
            "service_password",
            "CUSTOM_AUTH_VALUE",
            "AWS_SECRET_ACCESS_KEY",
        ] {
            assert!(is_secret_environment_key(key), "{key}");
        }
        assert!(!is_secret_environment_key("PATH"));
        assert!(!is_secret_environment_key("GWT_PROJECT_ROOT"));
        let mut configured = BTreeSet::new();
        add_secret_environment_values(
            &mut configured,
            [
                (
                    "CUSTOM_PASSWORD".to_string(),
                    "configured-value".to_string(),
                ),
                ("PATH".to_string(), "/public/bin".to_string()),
            ],
        );
        assert!(configured.contains("configured-value"));
        assert!(!configured.contains("/public/bin"));
    }

    #[test]
    fn privacy_validator_rejects_dynamic_deny_values_without_echoing_them() {
        for denied in [
            "acme/private-repo",
            "/Users/alice/private-repo",
            "/Users/alice/private-repo-worktree",
            "/Users/alice",
            "alice-machine-user",
            "customer-8675309",
            "configured-secret-value",
            "xy",
        ] {
            let context = PublicMutationContext::from_denied_values_for_test([denied]);
            let error = validate_public_payload(
                &payload_with_body(&format!("Public body accidentally contains {denied}")),
                &context,
            )
            .expect_err("dynamic taint must fail closed");
            assert_eq!(error.kind(), PrivacyViolationKind::DynamicValue);
            assert!(!error.to_string().contains(denied));
        }
    }

    #[test]
    fn privacy_validator_rejects_replay_proof_values_in_typed_public_fields() {
        let mut candidate = candidate(true);
        let private_nonce = match candidate.distinct_occurrences[0]
            .replay_proof
            .as_ref()
            .expect("replay proof")
        {
            OccurrenceReplayProof::RegisteredEvent {
                source_scope_nonce, ..
            }
            | OccurrenceReplayProof::InterpretiveSession {
                source_scope_nonce, ..
            } => source_scope_nonce.clone(),
        };
        candidate
            .typed_evidence
            .as_mut()
            .expect("typed evidence")
            .expected_outcome = private_nonce.clone();
        let evidence = candidate.typed_evidence.as_ref().expect("typed evidence");
        let fingerprint = improvement_fingerprint(evidence);
        let evidence_digest = typed_evidence_digest(evidence);
        candidate.fingerprint = Some(fingerprint.clone());
        let occurrence = &mut candidate.distinct_occurrences[0];
        occurrence.evidence_digest = evidence_digest;
        let (source_scope_nonce, source_event_id) =
            match occurrence.replay_proof.as_ref().expect("replay proof") {
                OccurrenceReplayProof::RegisteredEvent {
                    source_scope_nonce,
                    source_event_id,
                } => (source_scope_nonce, source_event_id),
                OccurrenceReplayProof::InterpretiveSession { .. } => {
                    panic!("expected registered event proof")
                }
            };
        occurrence.opaque_key = opaque_occurrence_key(
            source_scope_nonce,
            &fingerprint,
            occurrence.producer_id.as_deref().expect("producer"),
            source_event_id,
        );

        let error = render_public_issue_payload(&candidate, &PublicMutationContext::default())
            .expect_err("private replay proof value must fail closed");

        assert_eq!(error.kind(), PrivacyViolationKind::DynamicValue);
        assert!(!error.to_string().contains(private_nonce.as_str()));
    }

    #[test]
    fn privacy_validator_rejects_structural_taint_and_size_matrix() {
        let context = PublicMutationContext::from_denied_values_for_test([] as [&str; 0]);
        let cases = [
            ("read /var/tmp/private.log", PrivacyViolationKind::UnixPath),
            (
                "read C:\\Users\\alice\\private.log",
                PrivacyViolationKind::WindowsPath,
            ),
            (
                "token ghp_abcdefghijklmnopqrstuvwxyz",
                PrivacyViolationKind::Secret,
            ),
            (
                "Authorization: Bearer opaque-value",
                PrivacyViolationKind::Authorization,
            ),
            (
                "https://alice:password@example.com/path",
                PrivacyViolationKind::UrlCredential,
            ),
            (
                "https://example.com/path?access_token=opaque-value",
                PrivacyViolationKind::UrlQuerySecret,
            ),
            (
                "-----BEGIN OPENSSH PRIVATE KEY-----",
                PrivacyViolationKind::PrivateKey,
            ),
            ("contact alice@example.com", PrivacyViolationKind::Email),
            (
                "2026-07-14T00:00:00Z ERROR request failed\nstack backtrace:",
                PrivacyViolationKind::LogExcerpt,
            ),
            (
                "```rust\nfn private() {}\n```",
                PrivacyViolationKind::CodeExcerpt,
            ),
        ];
        for (body, expected) in cases {
            let error = validate_public_payload(&payload_with_body(body), &context)
                .expect_err("structural taint must fail closed");
            assert_eq!(error.kind(), expected, "body: {body}");
        }

        let oversized = "x".repeat(MAX_PUBLIC_BODY_BYTES + 1);
        let error = validate_public_payload(&payload_with_body(&oversized), &context)
            .expect_err("oversized payload must fail closed");
        assert_eq!(error.kind(), PrivacyViolationKind::SizeLimit);
    }

    #[test]
    fn incomplete_dynamic_privacy_context_fails_closed_before_owner_transport() {
        let root = tempfile::tempdir().expect("root");
        let missing_repo = root.path().join("missing-repository");
        let context = PublicMutationContext::for_repo(&missing_repo);
        assert!(render_public_issue_payload(&candidate(true), &context).is_err());
        let expired_deadline = ResolutionDeadline::at(
            Instant::now() - Duration::from_millis(1),
            Duration::from_secs(1),
        );
        let expired_context =
            PublicMutationContext::for_repo_with_deadline(root.path(), &expired_deadline);
        assert!(render_public_issue_payload(&candidate(true), &expired_context).is_err());

        let mut env = TestEnv::new(root.path().join("cache"));
        env.repo_path = missing_repo;
        let mut privacy_candidate = candidate(true);
        let outcome = owner_resolution_preflight(
            &mut env,
            &mut privacy_candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &resolution_deadline(),
        )
        .expect("privacy blocked outcome");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::Blocked {
                reason: BlockedReason::Privacy,
                ..
            }
        ));
        assert!(env.owner_client.owner_call_log().is_empty());
        assert!(board_bodies(&mut env)
            .last()
            .is_some_and(|body| body.contains("Reason: privacy/none")));

        let mut expired_env = TestEnv::new(root.path().join("expired-cache"));
        expired_env.repo_path = root.path().to_path_buf();
        let mut expired_candidate = candidate(true);
        let expired_outcome = owner_resolution_preflight(
            &mut expired_env,
            &mut expired_candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &expired_deadline,
        )
        .expect("timeout blocked outcome");
        assert!(matches!(
            expired_outcome,
            OwnerPreflightOutcome::Blocked {
                reason: BlockedReason::Timeout,
                ..
            }
        ));
        assert!(expired_env.owner_client.owner_call_log().is_empty());
    }

    #[test]
    fn typed_and_legacy_templates_never_render_free_form_candidate_fields() {
        let context = PublicMutationContext::from_denied_values_for_test([
            "customer-8675309",
            "/Users/alice/private-repo",
            "ghp_abcdefghijklmnopqrstuvwxyz",
            "alice@example.com",
        ]);
        let typed = render_public_issue_payload(&candidate(true), &context)
            .expect("typed contract template");
        assert_eq!(typed.summary, "coordination: STATUS_NOT_POSTED");
        assert!(typed.body.contains("coordination.board-status"));
        assert!(typed.body.contains("BOARD_STATUS_POSTED"));
        assert!(typed.body.contains("BOARD_STATUS_MISSING"));
        assert!(!typed.body.contains("customer-8675309"));
        assert!(!typed.body.contains("private-repo"));
        assert!(!typed.body.contains("ghp_"));
        assert!(!typed.body.contains("alice@example.com"));

        let legacy = render_public_issue_payload(&candidate(false), &context)
            .expect("fixed legacy template");
        assert_eq!(
            legacy.summary,
            "Self-improvement contract evidence required"
        );
        assert!(!legacy.body.contains("customer-8675309"));
        assert!(!legacy.body.contains("private-repo"));
    }

    #[test]
    fn typed_issue_template_contains_computed_digest_and_exact_fingerprint_marker() {
        let context = PublicMutationContext::default();
        let payload = render_public_issue_payload(&candidate(true), &context)
            .expect("typed contract template");
        assert!(payload.body.contains(
            "Public evidence digest: sha256:3f649bd386b953b42442e8cefcbd1449d657f49a972f11d72f810bcda167756a"
        ));
        let marker = "<!-- gwt:improvement-fingerprint:v1 v2:4bea839977a5aeedbf562acaeeb547012b0447f3335279830405fafb37726532 -->";
        assert!(payload.body.lines().any(|line| line == marker));

        let lookalikes = format!(
            "{marker}\n{marker} suffix\nprefix {marker}\n```\n{marker}\n```\n~~~markdown\n{marker}\n~~~\n<!-- gwt:improvement-fingerprint:v1 v2:{} -->",
            "0".repeat(64)
        );
        assert_eq!(
            exact_fingerprint_markers(&lookalikes),
            vec![
                "v2:4bea839977a5aeedbf562acaeeb547012b0447f3335279830405fafb37726532".to_string(),
                format!("v2:{}", "0".repeat(64)),
            ]
        );
    }

    #[test]
    fn occurrence_and_reconciliation_templates_are_opaque_and_taint_free() {
        let candidate = candidate(true);
        let context = PublicMutationContext::from_denied_values_for_test([
            "customer-8675309",
            "/Users/alice/private-repo",
            "ghp_abcdefghijklmnopqrstuvwxyz",
            "alice@example.com",
        ]);
        let occurrence_key =
            "occ:v1:760fc151831a9d5bf11893e402fdf5d63727e188dbc17015c67b2054f4a97148";
        let occurrence = render_occurrence_comment_payload(&candidate, occurrence_key, &context)
            .expect("occurrence comment");
        assert!(occurrence.body.lines().any(|line| {
            line == "<!-- gwt:improvement-occurrence:v1 occ:v1:760fc151831a9d5bf11893e402fdf5d63727e188dbc17015c67b2054f4a97148 -->"
        }));
        assert!(occurrence.body.contains("Public evidence digest: sha256:"));

        let reconciliation = render_reconciliation_comment_payload(&candidate, 42, 84, &context)
            .expect("reconciliation comment");
        assert!(reconciliation.body.contains("Canonical owner: #42"));
        assert!(reconciliation.body.contains("Duplicate owner: #84"));
        assert!(reconciliation.body.lines().any(|line| {
            line == "<!-- gwt:improvement-reconciliation:v1 canonical:42 duplicate:84 -->"
        }));

        for body in [&occurrence.body, &reconciliation.body] {
            assert!(!body.contains("customer-8675309"));
            assert!(!body.contains("private-repo"));
            assert!(!body.contains("ghp_"));
            assert!(!body.contains("alice@example.com"));
        }
    }

    #[test]
    fn direct_stop_unhandled_failure_is_persisted_as_blocked() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let repo_root = project.path();
        let initial = candidate(true);
        let candidate_id = initial.id.clone();
        crate::cli::improvement_store::update(repo_root, |store| {
            store.candidates.push(initial);
            Ok(())
        })
        .expect("seed candidate");
        let ResolutionAttemptStart::Acquired { token, .. } =
            begin_resolution_attempt(repo_root, &candidate_id, CaptureBudgetProfile::StrictStop)
                .expect("acquire direct Stop attempt");
        let failure = OwnerResolutionFailure {
            reason: BlockedReason::Timeout,
            failure_subcode: None,
            remediation: "RETRY_WITHIN_BUDGET",
        };

        settle_unhandled_resolution_failure(repo_root, &token, failure)
            .expect("persist direct Stop failure");

        let stored = crate::cli::improvement_store::load_and_repair(repo_root)
            .expect("load candidate")
            .candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id)
            .expect("stored candidate");
        assert_eq!(stored.state, CandidateState::Blocked);
        assert_eq!(stored.blocked_reason, Some(BlockedReason::Timeout));
        assert!(stored.attempt.is_none());
        assert_eq!(
            stored.retry.expect("retry metadata").remediation,
            "RETRY_WITHIN_BUDGET"
        );
    }

    #[test]
    fn unhandled_settlement_requires_the_exact_attempt_id() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let repo_root = project.path();
        let initial = candidate(true);
        let candidate_id = initial.id.clone();
        crate::cli::improvement_store::update(repo_root, |store| {
            store.candidates.push(initial);
            Ok(())
        })
        .expect("seed candidate");
        let ResolutionAttemptStart::Acquired { token, .. } =
            begin_resolution_attempt(repo_root, &candidate_id, CaptureBudgetProfile::StrictStop)
                .expect("acquire first attempt");
        let mut wrong_token = token.clone();
        wrong_token.attempt_id = "attempt-other-worker".to_string();
        let failure = OwnerResolutionFailure {
            reason: BlockedReason::Store,
            failure_subcode: None,
            remediation: "RELOAD_CANDIDATE_STORE",
        };

        let settled = settle_unhandled_resolution_failure(repo_root, &wrong_token, failure)
            .expect("stale settlement is a no-op");

        assert!(!settled);
        let stored = crate::cli::improvement_store::load_and_repair(repo_root)
            .expect("load candidate")
            .candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id)
            .expect("stored candidate");
        assert_eq!(stored.state, CandidateState::OwnerResolving);
        assert_eq!(
            stored.attempt.expect("active attempt").attempt_id,
            token.attempt_id
        );
    }

    #[test]
    fn unhandled_failure_after_a_completed_remote_step_stays_remote_unknown() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let repo_root = project.path();
        let initial = candidate(true);
        let candidate_id = initial.id.clone();
        crate::cli::improvement_store::update(repo_root, |store| {
            store.candidates.push(initial);
            Ok(())
        })
        .expect("seed candidate");
        let ResolutionAttemptStart::Acquired { token, .. } =
            begin_resolution_attempt(repo_root, &candidate_id, CaptureBudgetProfile::StrictStop)
                .expect("acquire direct Stop attempt");
        let intent = ResolutionAttemptIntent::CloseDuplicate {
            canonical_owner_number: 41,
            duplicate_owner_number: 42,
        };
        crate::cli::improvement_store::update(repo_root, |store| {
            crate::cli::improvement_store::mark_attempt_submitted(
                store,
                &candidate_id,
                &token.attempt_id,
                intent.clone(),
            )?;
            crate::cli::improvement_store::complete_attempt_step(
                store,
                &candidate_id,
                &token.attempt_id,
                &intent,
            )?;
            Ok(())
        })
        .expect("complete one remote mutation step");
        let failure = OwnerResolutionFailure {
            reason: BlockedReason::Store,
            failure_subcode: None,
            remediation: "RELOAD_CANDIDATE_STORE",
        };

        settle_unhandled_resolution_failure(repo_root, &token, failure)
            .expect("persist remote uncertainty");

        let stored = crate::cli::improvement_store::load_and_repair(repo_root)
            .expect("load candidate")
            .candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id)
            .expect("stored candidate");
        assert_eq!(stored.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(stored.blocked_reason, None);
        assert_eq!(
            stored.retry.expect("retry metadata").remediation,
            "REFRESH_OWNER_CORPUS"
        );
    }

    #[test]
    fn stale_attempt_token_cannot_overwrite_takeover_state() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let repo_root = project.path();
        let initial = candidate(true);
        let candidate_id = initial.id.clone();
        crate::cli::improvement_store::update(repo_root, |store| {
            store.candidates.push(initial);
            Ok(())
        })
        .expect("seed candidate");
        let ResolutionAttemptStart::Acquired {
            candidate: first_candidate,
            token: first_token,
            ..
        } = begin_resolution_attempt(repo_root, &candidate_id, CaptureBudgetProfile::Normal)
            .expect("first attempt");
        crate::cli::improvement_store::update(repo_root, |store| {
            let candidate = store
                .candidates
                .iter_mut()
                .find(|candidate| candidate.id == candidate_id)
                .expect("candidate");
            candidate.attempt.as_mut().expect("attempt").expires_at =
                Utc::now() - chrono::Duration::seconds(1);
            Ok(())
        })
        .expect("expire first attempt");
        let ResolutionAttemptStart::Acquired {
            token: second_token,
            ..
        } = begin_resolution_attempt(repo_root, &candidate_id, CaptureBudgetProfile::Normal)
            .expect("takeover attempt");
        assert_ne!(first_token.attempt_id, second_token.attempt_id);
        let mut stale = first_candidate;
        stale.blocked_reason = Some(BlockedReason::Store);
        stale.retry = Some(RetryMetadata {
            retryable: true,
            remediation: "RELOAD_CANDIDATE_STORE".to_string(),
            failed_at: Utc::now().to_rfc3339(),
        });
        transition_candidate(&mut stale, CandidateState::Blocked).expect("stale blocked state");
        stale.attempt = None;

        let error = persist_resolver_candidate(repo_root, &stale, &first_token)
            .expect_err("stale worker must not persist");

        assert!(error.to_string().contains("attempt lease is stale"));
        let stored = crate::cli::improvement_store::load_and_repair(repo_root)
            .expect("stored takeover candidate")
            .candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id)
            .expect("stored candidate");
        assert_eq!(stored.state, CandidateState::OwnerResolving);
        assert_eq!(
            stored.attempt.expect("takeover lease").attempt_id,
            second_token.attempt_id
        );
    }

    #[test]
    fn renewed_worker_cannot_commit_active_owner_after_late_takeover() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let candidate_id = "impr-stale-active-success";
        let (fingerprint, occurrence_key) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "stale-success");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            44,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        let ResolutionAttemptStart::Acquired {
            candidate: mut first_candidate,
            token: first_token,
            ..
        } = begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
            .expect("first attempt");
        renew_resolution_attempt(source.path(), &mut first_candidate, &first_token)
            .expect("renew first attempt before suspension");
        let comment_body = render_occurrence_comment_payload(
            &first_candidate,
            &occurrence_key,
            &PublicMutationContext::for_repo(source.path()),
        )
        .expect("occurrence payload")
        .body;
        let comment = repository_comment(1, comment_body, "2026-07-15T10:00:00Z");
        env.owner_client.seed_repository_comments(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(44),
            vec![comment.clone()],
        );
        crate::cli::improvement_store::update(source.path(), |store| {
            let candidate = store
                .candidates
                .iter_mut()
                .find(|candidate| candidate.id == candidate_id)
                .expect("candidate");
            candidate.attempt.as_mut().expect("attempt").expires_at =
                Utc::now() - chrono::Duration::seconds(1);
            Ok(())
        })
        .expect("expire first attempt");
        let ResolutionAttemptStart::Acquired {
            token: second_token,
            ..
        } = begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
            .expect("takeover attempt");

        let result = commit_active_owner_resolution(
            &mut env,
            &mut first_candidate,
            &OwnerCandidate {
                number: 44,
                kind: OwnerKind::Issue,
                title: "Owner 44".to_string(),
                active: true,
                url: "https://github.com/akiojin/gwt/issues/44".to_string(),
                match_basis: OwnerMatchBasis::Fingerprint,
                selectable: true,
            },
            &[comment],
            &resolution_deadline(),
        );

        assert!(
            result.is_err(),
            "stale success path must fail its lease CAS"
        );
        let stored = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("stored takeover candidate")
            .candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id)
            .expect("stored candidate");
        assert_eq!(stored.state, CandidateState::OwnerResolving);
        assert_eq!(
            stored.attempt.expect("takeover lease").attempt_id,
            second_token.attempt_id
        );
        assert!(crate::cli::improvement_store::load_owner_projection()
            .expect("owner projection")
            .owners
            .is_empty());
    }

    #[test]
    fn stale_attempt_cannot_write_projection_inside_source_transaction() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let candidate_id = "impr-projection-transaction-fence";
        let (fingerprint, occurrence_key) = store_projection_candidate(
            source.path(),
            &nonce,
            candidate_id,
            "projection-transaction-fence",
        );
        let ResolutionAttemptStart::Acquired {
            candidate: first_candidate,
            token: first_token,
            ..
        } = begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
            .expect("first attempt");
        let commit = prepare_test_owner_projection_commit(
            source.path(),
            &ReadbackVerifiedOwnerBinding {
                candidate_id: candidate_id.to_string(),
                owner: verified_projection_owner(44, &fingerprint),
                occurrence_key,
                resolution_status: CandidateState::Linked,
                last_seen: first_candidate.distinct_occurrences[0].captured_at.clone(),
            },
        )
        .expect("prepare projection commit");
        crate::cli::improvement_store::update(source.path(), |store| {
            store.candidates[0]
                .attempt
                .as_mut()
                .expect("attempt")
                .expires_at = Utc::now() - chrono::Duration::seconds(1);
            Ok(())
        })
        .expect("expire first attempt");
        let ResolutionAttemptStart::Acquired {
            token: second_token,
            ..
        } = begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
            .expect("takeover attempt");

        let error = commit_owner_projection_and_source_success(
            source.path(),
            &first_candidate,
            vec![commit],
            44,
            false,
        )
        .expect_err("stale projection transaction must lose its source lease fence");

        assert!(error.to_string().contains("attempt lease is stale"));
        assert!(crate::cli::improvement_store::load_owner_projection()
            .expect("owner projection")
            .owners
            .is_empty());
        let stored = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("stored takeover candidate")
            .candidates
            .remove(0);
        assert_eq!(stored.state, CandidateState::OwnerResolving);
        assert_eq!(
            stored.attempt.expect("takeover lease").attempt_id,
            second_token.attempt_id
        );
        assert_ne!(first_token.attempt_id, second_token.attempt_id);
    }

    #[test]
    fn projection_batch_failure_does_not_persist_a_prefix() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let candidate_id = "impr-atomic-projection-batch";
        let (fingerprint, first_occurrence_key) = store_projection_candidate(
            source.path(),
            &nonce,
            candidate_id,
            "atomic-projection-first",
        );
        let second_occurrence_key =
            append_projection_occurrence(source.path(), &nonce, "atomic-projection-second");
        let ResolutionAttemptStart::Acquired { candidate, .. } =
            begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
                .expect("resolution attempt");
        let owner = verified_projection_owner(44, &fingerprint);
        let first_commit = prepare_test_owner_projection_commit(
            source.path(),
            &ReadbackVerifiedOwnerBinding {
                candidate_id: candidate_id.to_string(),
                owner: owner.clone(),
                occurrence_key: first_occurrence_key,
                resolution_status: CandidateState::Linked,
                last_seen: candidate.distinct_occurrences[0].captured_at.clone(),
            },
        )
        .expect("first projection commit");
        let mut invalid_second_commit = prepare_test_owner_projection_commit(
            source.path(),
            &ReadbackVerifiedOwnerBinding {
                candidate_id: candidate_id.to_string(),
                owner,
                occurrence_key: second_occurrence_key,
                resolution_status: CandidateState::Linked,
                last_seen: candidate.distinct_occurrences[1].captured_at.clone(),
            },
        )
        .expect("second projection commit");
        invalid_second_commit.occurrence.last_seen = "not-an-rfc3339-timestamp".to_string();

        let error = commit_owner_projection_and_source_success(
            source.path(),
            &candidate,
            vec![first_commit, invalid_second_commit],
            44,
            false,
        )
        .expect_err("an invalid later commit must abort the full projection batch");

        assert!(error.to_string().contains("last_seen must be RFC3339"));
        assert!(
            crate::cli::improvement_store::load_owner_projection()
                .expect("projection after failed batch")
                .owners
                .is_empty(),
            "no valid prefix may survive a failed projection batch"
        );
        let stored = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source after failed batch")
            .candidates
            .remove(0);
        assert_eq!(stored.state, CandidateState::OwnerResolving);
        assert!(stored.owner.is_none());
    }

    #[test]
    fn expired_attempt_repairs_projection_first_crash_before_takeover() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let candidate_id = "impr-expired-projection-repair";
        let (fingerprint, occurrence_key) = store_projection_candidate(
            source.path(),
            &nonce,
            candidate_id,
            "expired-projection-repair",
        );
        let ResolutionAttemptStart::Acquired { candidate, .. } =
            begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
                .expect("resolution attempt");
        commit_readback_verified_binding(
            source.path(),
            &ReadbackVerifiedOwnerBinding {
                candidate_id: candidate_id.to_string(),
                owner: verified_projection_owner(44, &fingerprint),
                occurrence_key,
                resolution_status: CandidateState::Linked,
                last_seen: candidate.distinct_occurrences[0].captured_at.clone(),
            },
        )
        .expect("simulate projection-first persist before crash");
        crate::cli::improvement_store::update(source.path(), |store| {
            store.candidates[0]
                .attempt
                .as_mut()
                .expect("attempt")
                .expires_at = Utc::now() - chrono::Duration::seconds(1);
            Ok(())
        })
        .expect("expire crashed attempt");

        assert!(
            repair_source_success_snapshots(source.path()).expect("repair expired attempt"),
            "projection-first crash must converge before another worker takes over"
        );
        let repaired = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("repaired source")
            .candidates
            .remove(0);
        assert_eq!(repaired.state, CandidateState::Linked);
        assert_eq!(repaired.owner.as_ref().unwrap().number, 44);
        assert!(repaired.attempt.is_none());
    }

    #[test]
    fn duplicate_reconciliation_revalidates_exact_owners_before_mutation() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let candidate_id = "impr-owner-fixture-991";
        let (fingerprint, _) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "source-event-772");
        crate::cli::improvement_store::update(source.path(), |store| {
            let candidate = store.candidates.first_mut().expect("candidate");
            candidate.dedupe_key = format!("fingerprint:{fingerprint}");
            candidate.sanitized_summary = "Safe typed candidate".to_string();
            candidate.sanitized_details = None;
            candidate.evidence_digest = Some(typed_evidence_digest(
                candidate.typed_evidence.as_ref().expect("typed evidence"),
            ));
            candidate.local_evidence.clear();
            Ok(())
        })
        .expect("sanitize source fixture");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        let stored_fixture = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("fixture candidate")
            .candidates
            .remove(0);
        render_public_issue_payload(
            &stored_fixture,
            &PublicMutationContext::for_repo(source.path()),
        )
        .unwrap_or_else(|error| panic!("unsafe fixture issue payload: {error}"));
        for occurrence in &stored_fixture.distinct_occurrences {
            render_occurrence_comment_payload(
                &stored_fixture,
                &occurrence.opaque_key,
                &PublicMutationContext::for_repo(source.path()),
            )
            .unwrap_or_else(|error| panic!("unsafe fixture occurrence payload: {error}"));
        }
        for number in [44, 45] {
            env.owner_client.seed_repository_issue(owner_issue(
                number,
                IssueState::Open,
                fingerprint_marker(&fingerprint),
            ));
        }
        let ResolutionAttemptStart::Acquired {
            mut candidate,
            token,
            ..
        } = begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
            .expect("resolution attempt");
        let outcome = owner_resolution_preflight_deferred(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::current(),
            &NoSemanticOwnerAdvisor,
            &resolution_deadline(),
        )
        .expect("duplicate preflight");
        let OwnerPreflightOutcome::DuplicateExactIssues {
            owners,
            comments_by_owner,
            ..
        } = outcome
        else {
            panic!("expected duplicate exact owners, got {outcome:?}")
        };
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Open,
            "fingerprint marker removed after preflight",
        ));

        let result = reconcile_duplicate_owners(
            &mut env,
            &mut candidate,
            &token,
            owners,
            comments_by_owner,
            &resolution_deadline(),
        );

        assert!(result.is_err(), "changed duplicate must fail closed");
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        let duplicate = env
            .owner_client
            .fetch_issue(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(45),
                &resolution_deadline(),
            )
            .expect("duplicate readback");
        assert_eq!(duplicate.state, IssueState::Open);
    }

    #[test]
    fn owner_preflight_finds_body_marker_then_fully_reads_only_the_exact_owner_comments() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        let candidate = candidate(true);
        let repository = RepositoryIdentity::gwt_upstream();
        env.owner_client.seed_repository_issue(owner_issue(
            42,
            IssueState::Open,
            fingerprint_marker(candidate.fingerprint.as_deref().expect("fingerprint")),
        ));
        let mut comments = (1..=204)
            .map(|id| repository_comment(id, format!("public comment {id}"), format!("c{id}")))
            .collect::<Vec<_>>();
        comments.push(repository_comment(
            205,
            occurrence_marker(&candidate.distinct_occurrences[0].opaque_key),
            "c205",
        ));
        env.owner_client
            .seed_repository_comments(&repository, IssueNumber(42), comments);

        let mut candidate = candidate;
        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &resolution_deadline(),
        )
        .expect("preflight");
        match outcome {
            OwnerPreflightOutcome::Active { owner, .. } => {
                assert_eq!(owner.number, 42);
                assert_eq!(owner.match_basis, OwnerMatchBasis::Fingerprint);
            }
            other => panic!("expected active owner, got {other:?}"),
        }
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        let calls = env.owner_client.owner_call_log();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.operation == OwnerRepositoryOperation::ListIssues)
                .count(),
            2
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.operation == OwnerRepositoryOperation::ListComments)
                .count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .find(|call| call.operation == OwnerRepositoryOperation::ListComments)
                .and_then(|call| call.issue_number),
            Some(IssueNumber(42))
        );
    }

    #[test]
    fn owner_preflight_matches_only_the_candidate_fingerprint_on_a_shared_owner() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        let candidate = candidate(true);
        let candidate_marker =
            fingerprint_marker(candidate.fingerprint.as_deref().expect("fingerprint"));
        let other_marker = fingerprint_marker(&format!("v2:{}", "0".repeat(64)));
        env.owner_client.seed_repository_issue(owner_issue(
            42,
            IssueState::Open,
            format!("{candidate_marker}\n{other_marker}"),
        ));

        let mut candidate = candidate;
        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &StaticSemanticAdvisor(Ok(Vec::new())),
            &resolution_deadline(),
        )
        .expect("preflight");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::Active { owner, .. }
                if owner.number == 42 && owner.match_basis == OwnerMatchBasis::Fingerprint
        ));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
    }

    #[test]
    fn owner_preflight_zero_confirms_two_stable_issue_generations_without_comment_n_plus_one() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        let repository = RepositoryIdentity::gwt_upstream();
        for number in 1..=125 {
            env.owner_client.seed_repository_issue(owner_issue(
                number,
                IssueState::Open,
                "unrelated public owner",
            ));
        }
        env.owner_client
            .queue_owner_issue_generations(&repository, ["generation-stable", "generation-stable"]);
        let mut candidate = candidate(true);

        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &implementation_gap_registry(1),
            &StaticSemanticAdvisor(Ok(Vec::new())),
            &resolution_deadline(),
        )
        .expect("preflight");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::CreateAuthorized { authorization }
                if !authorization.corpus_generation.is_empty()
        ));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        let calls = env.owner_client.owner_call_log();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.operation == OwnerRepositoryOperation::ListIssues)
                .count(),
            2
        );
        assert!(!calls
            .iter()
            .any(|call| call.operation == OwnerRepositoryOperation::ListComments));
    }

    #[test]
    fn owner_preflight_fails_closed_when_issue_generation_changes_during_resolution() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        let repository = RepositoryIdentity::gwt_upstream();
        env.owner_client.seed_repository_issue(owner_issue(
            7,
            IssueState::Closed,
            "unrelated public issue",
        ));
        env.owner_client
            .queue_owner_issue_generations(&repository, ["generation-before", "generation-after"]);
        let mut candidate = candidate(true);

        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &implementation_gap_registry(1),
            &StaticSemanticAdvisor(Ok(Vec::new())),
            &resolution_deadline(),
        )
        .expect("preflight");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::Blocked {
                reason: BlockedReason::Search,
                failure_subcode: Some(FailureSubcode::PartialPage)
            }
        ));
        let calls = env.owner_client.owner_call_log();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.operation == OwnerRepositoryOperation::ListIssues)
                .count(),
            2
        );
        assert!(!calls
            .iter()
            .any(|call| call.operation == OwnerRepositoryOperation::ListComments));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        assert!(board_bodies(&mut env)
            .last()
            .is_some_and(|body| body.contains("Reason: search/partial-page")));
    }

    #[test]
    fn owner_preflight_rejects_noncanonical_ineligible_and_private_candidates_before_remote_read() {
        for case in ["fingerprint", "eligibility", "privacy"] {
            let project = tempfile::tempdir().expect("project");
            let mut env = TestEnv::new(project.path().join("cache"));
            env.repo_path = project.path().to_path_buf();
            let mut candidate = candidate(true);
            let expected_reason = match case {
                "fingerprint" => {
                    candidate.fingerprint = Some(format!("v2:{}", "0".repeat(64)));
                    BlockedReason::Routing
                }
                "eligibility" => {
                    candidate.eligibility =
                        super::super::improvement::ImprovementEligibility::NeedsEvidence;
                    BlockedReason::Routing
                }
                "privacy" => {
                    let evidence = candidate.typed_evidence.as_mut().expect("typed evidence");
                    evidence.expected_outcome = "alice@example.com".to_string();
                    let fingerprint = improvement_fingerprint(evidence);
                    let evidence_digest = typed_evidence_digest(evidence);
                    candidate.fingerprint = Some(fingerprint.clone());
                    let occurrence = &mut candidate.distinct_occurrences[0];
                    occurrence.evidence_digest = evidence_digest;
                    let OccurrenceReplayProof::RegisteredEvent {
                        source_scope_nonce,
                        source_event_id,
                    } = occurrence.replay_proof.as_ref().expect("replay proof")
                    else {
                        panic!("registered replay proof");
                    };
                    occurrence.opaque_key = opaque_occurrence_key(
                        source_scope_nonce,
                        &fingerprint,
                        occurrence.producer_id.as_deref().expect("producer"),
                        source_event_id,
                    );
                    BlockedReason::Privacy
                }
                _ => unreachable!(),
            };

            let outcome = owner_resolution_preflight(
                &mut env,
                &mut candidate,
                &ContractRoutingRegistry::default(),
                &StaticSemanticAdvisor(Ok(Vec::new())),
                &resolution_deadline(),
            )
            .expect("preflight");

            assert!(matches!(
                outcome,
                OwnerPreflightOutcome::Blocked { reason, .. } if reason == expected_reason
            ));
            assert!(env.owner_client.owner_call_log().is_empty(), "{case}");
            assert_eq!(env.owner_client.owner_mutation_count(), 0, "{case}");
            let reason = blocked_reason_token(expected_reason);
            assert!(board_bodies(&mut env).last().is_some_and(|body| {
                body.contains(candidate.id.as_str())
                    && body.contains(format!("Reason: {reason}/none").as_str())
            }));
        }
    }

    #[test]
    fn owner_preflight_rejects_tampered_producer_and_duplicate_interpretive_occurrences() {
        for case in [
            "missing-producer",
            "stale-registry",
            "unregistered-producer",
            "mismatched-evidence-digest",
            "duplicate-interpretive",
            "forged-distinct-interpretive",
        ] {
            let project = tempfile::tempdir().expect("project");
            let mut env = TestEnv::new(project.path().join("cache"));
            env.repo_path = project.path().to_path_buf();
            let mut candidate = candidate(true);
            match case {
                "missing-producer" => candidate.distinct_occurrences[0].producer_id = None,
                "stale-registry" => {
                    candidate.distinct_occurrences[0].producer_registry_revision = Some(999)
                }
                "unregistered-producer" => {
                    candidate.distinct_occurrences[0].producer_id = Some("unknown.v1".to_string())
                }
                "mismatched-evidence-digest" => {
                    candidate.distinct_occurrences[0].evidence_digest = "b".repeat(64)
                }
                "duplicate-interpretive" => {
                    let source_scope_nonce = "0".repeat(64);
                    let session_id = "00000000-0000-4000-8000-000000000001";
                    let fingerprint = candidate.fingerprint.clone().expect("fingerprint");
                    let occurrence = &mut candidate.distinct_occurrences[0];
                    occurrence.origin = OccurrenceOrigin::Interpretive;
                    occurrence.producer_id = None;
                    occurrence.producer_registry_revision = None;
                    occurrence.routing_basis_revision = None;
                    occurrence.opaque_key = opaque_occurrence_key(
                        &source_scope_nonce,
                        &fingerprint,
                        "json.interpretive",
                        session_id,
                    );
                    occurrence.replay_proof = Some(OccurrenceReplayProof::InterpretiveSession {
                        source_scope_nonce,
                        session_id: session_id.to_string(),
                    });
                    let duplicate = occurrence.clone();
                    candidate.distinct_occurrences.push(duplicate);
                    candidate.occurrences = 2;
                    candidate.eligibility = ImprovementEligibility::InterpretiveCorroboration;
                }
                "forged-distinct-interpretive" => {
                    let source_scope_nonce = "0".repeat(64);
                    let first_session = "00000000-0000-4000-8000-000000000001";
                    let second_session = "00000000-0000-4000-8000-000000000002";
                    let fingerprint = candidate.fingerprint.clone().expect("fingerprint");
                    let occurrence = &mut candidate.distinct_occurrences[0];
                    occurrence.origin = OccurrenceOrigin::Interpretive;
                    occurrence.producer_id = None;
                    occurrence.producer_registry_revision = None;
                    occurrence.routing_basis_revision = None;
                    occurrence.opaque_key = opaque_occurrence_key(
                        &source_scope_nonce,
                        &fingerprint,
                        "json.interpretive",
                        first_session,
                    );
                    occurrence.replay_proof = Some(OccurrenceReplayProof::InterpretiveSession {
                        source_scope_nonce: source_scope_nonce.clone(),
                        session_id: first_session.to_string(),
                    });
                    let mut forged = occurrence.clone();
                    forged.opaque_key = opaque_occurrence_key(
                        &source_scope_nonce,
                        &fingerprint,
                        "json.interpretive",
                        second_session,
                    );
                    forged.replay_proof = Some(OccurrenceReplayProof::InterpretiveSession {
                        source_scope_nonce,
                        session_id: second_session.to_string(),
                    });
                    candidate.distinct_occurrences.push(forged);
                    candidate.occurrences = 2;
                    candidate.eligibility = ImprovementEligibility::InterpretiveCorroboration;
                }
                _ => unreachable!(),
            }

            let outcome = owner_resolution_preflight(
                &mut env,
                &mut candidate,
                &ContractRoutingRegistry::default(),
                &NoSemanticOwnerAdvisor,
                &resolution_deadline(),
            )
            .expect("tampered candidate outcome");

            assert!(
                matches!(
                    outcome,
                    OwnerPreflightOutcome::Blocked {
                        reason: BlockedReason::Routing,
                        ..
                    }
                ),
                "{case}: {outcome:?}"
            );
            assert!(env.owner_client.owner_call_log().is_empty(), "{case}");
            assert!(board_bodies(&mut env).last().is_some_and(|body| {
                body.contains(candidate.id.as_str()) && body.contains("Reason: routing/none")
            }));
        }
    }

    #[test]
    fn owner_preflight_rejects_replay_proof_outside_canonical_source_scope() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        let mut candidate = candidate(true);
        let forged_nonce = "f".repeat(64);
        let occurrence = &mut candidate.distinct_occurrences[0];
        let source_event_id = match occurrence.replay_proof.as_mut().expect("replay proof") {
            OccurrenceReplayProof::RegisteredEvent {
                source_scope_nonce,
                source_event_id,
            } => {
                *source_scope_nonce = forged_nonce.clone();
                source_event_id.clone()
            }
            OccurrenceReplayProof::InterpretiveSession { .. } => {
                panic!("expected registered event proof")
            }
        };
        occurrence.opaque_key = opaque_occurrence_key(
            &forged_nonce,
            candidate.fingerprint.as_deref().expect("fingerprint"),
            occurrence.producer_id.as_deref().expect("producer"),
            &source_event_id,
        );

        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &resolution_deadline(),
        )
        .expect("canonical source scope outcome");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::Blocked {
                reason: BlockedReason::Routing,
                ..
            }
        ));
        assert!(env.owner_client.owner_call_log().is_empty());
    }

    #[test]
    fn owner_preflight_preserves_remote_outcome_unknown_when_refresh_fails() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::ListIssues,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            GitHubApiError::Timeout {
                operation: "private-timeout".to_string(),
            },
        );
        let mut candidate = candidate(true);
        candidate.state = CandidateState::RemoteOutcomeUnknown;
        candidate.retry = Some(RetryMetadata {
            retryable: true,
            remediation: "REFRESH_OWNER_CORPUS".to_string(),
            failed_at: "2026-07-14T00:00:00Z".to_string(),
        });

        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &StaticSemanticAdvisor(Ok(Vec::new())),
            &resolution_deadline(),
        )
        .expect("preflight");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::RemoteOutcomeUnknown {
                failure_reason: BlockedReason::Timeout,
                ..
            }
        ));
        assert_eq!(candidate.state, CandidateState::RemoteOutcomeUnknown);
        assert!(candidate.blocked_reason.is_none());
        assert_eq!(
            candidate
                .retry
                .as_ref()
                .map(|retry| retry.remediation.as_str()),
            Some("REFRESH_OWNER_CORPUS")
        );
        let status = board_bodies(&mut env).pop().expect("remote outcome status");
        assert!(status.contains("remote-outcome-unknown"));
        assert!(!status.contains("owner resolution is blocked"));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
    }

    #[test]
    fn owner_inspection_fails_closed_when_corpus_changes_during_semantic_advice() {
        let client = FakeIssueClient::new();
        client.seed_repository_issue(owner_issue(7, IssueState::Closed, "unrelated public issue"));
        let candidate = candidate(true);
        let advisor = CorpusMutatingAdvisor {
            client: &client,
            issue: owner_issue(
                42,
                IssueState::Open,
                fingerprint_marker(candidate.fingerprint.as_deref().expect("fingerprint")),
            ),
        };

        let outcome = inspect_owner_corpus(
            &client,
            &candidate,
            &ContractRoutingRegistry::default(),
            &advisor,
            &resolution_deadline(),
        );

        assert!(matches!(
            outcome,
            Err(OwnerResolutionFailure {
                reason: BlockedReason::Search,
                failure_subcode: Some(FailureSubcode::PartialPage),
                ..
            })
        ));
        assert_eq!(client.owner_mutation_count(), 0);
        assert_eq!(
            client
                .owner_call_log()
                .iter()
                .filter(|call| call.operation == OwnerRepositoryOperation::ListIssues)
                .count(),
            2
        );
    }

    #[test]
    fn owner_preflight_blocks_empty_multiple_and_semantic_only_without_mutation() {
        let cases = ["empty", "multiple", "semantic-only"];
        for case in cases {
            let project = tempfile::tempdir().expect("project");
            let mut env = TestEnv::new(project.path().join("cache"));
            env.repo_path = project.path().to_path_buf();
            let mut candidate = candidate(true);
            let marker = fingerprint_marker(candidate.fingerprint.as_deref().expect("fingerprint"));
            let advisor = match case {
                "empty" => StaticSemanticAdvisor(Ok(Vec::new())),
                "multiple" => {
                    env.owner_client.seed_repository_issue(owner_issue(
                        41,
                        IssueState::Open,
                        marker.clone(),
                    ));
                    let mut spec = owner_issue(42, IssueState::Open, marker.clone());
                    spec.kind = RepositoryIssueKind::Spec;
                    spec.labels = vec!["gwt-spec".to_string()];
                    env.owner_client.seed_repository_issue(spec);
                    StaticSemanticAdvisor(Ok(Vec::new()))
                }
                "semantic-only" => {
                    env.owner_client.seed_repository_issue(owner_issue(
                        42,
                        IssueState::Open,
                        "no exact marker",
                    ));
                    StaticSemanticAdvisor(Ok(vec![IssueNumber(42)]))
                }
                _ => unreachable!(),
            };
            let outcome = owner_resolution_preflight(
                &mut env,
                &mut candidate,
                &ContractRoutingRegistry::default(),
                &advisor,
                &resolution_deadline(),
            )
            .expect("preflight");
            let OwnerPreflightOutcome::Blocked {
                reason,
                failure_subcode,
            } = outcome
            else {
                panic!("{case} must block")
            };
            if case == "empty" {
                assert_eq!(reason, crate::cli::improvement::BlockedReason::Search);
                assert_eq!(
                    failure_subcode,
                    Some(crate::cli::improvement::FailureSubcode::EmptyCorpus)
                );
            } else {
                assert_eq!(reason, crate::cli::improvement::BlockedReason::Ambiguity);
                assert_eq!(failure_subcode, None);
            }
            assert_eq!(env.owner_client.owner_mutation_count(), 0, "{case}");
            assert!(board_bodies(&mut env).last().is_some_and(|body| {
                body.contains(candidate.id.as_str())
                    && body.contains(match case {
                        "empty" => "Reason: search/empty-corpus",
                        "multiple" | "semantic-only" => "Reason: ambiguity/none",
                        _ => unreachable!(),
                    })
            }));
            if case != "empty" {
                assert!(!env
                    .owner_client
                    .owner_call_log()
                    .iter()
                    .any(|call| call.operation == OwnerRepositoryOperation::ListComments));
            }
        }
    }

    #[test]
    fn advisory_semantic_failure_does_not_block_complete_authoritative_zero() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        env.owner_client.seed_repository_issue(owner_issue(
            7,
            IssueState::Closed,
            "unrelated public issue",
        ));
        let mut candidate = candidate(true);
        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &implementation_gap_registry(1),
            &StaticSemanticAdvisor(Err(())),
            &resolution_deadline(),
        )
        .expect("preflight");
        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::CreateAuthorized { .. }
        ));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        assert!(board_bodies(&mut env)
            .iter()
            .any(|body| body.contains("ADVISORY_INDEX_UNAVAILABLE")));
    }

    #[test]
    fn semantic_advisor_receives_the_same_absolute_resolution_deadline() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        env.owner_client.seed_repository_issue(owner_issue(
            7,
            IssueState::Closed,
            "unrelated public issue",
        ));
        let deadline = resolution_deadline();
        let advisor = DeadlineRecordingAdvisor::default();
        let mut candidate = candidate(true);

        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &implementation_gap_registry(1),
            &advisor,
            &deadline,
        )
        .expect("preflight");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::CreateAuthorized { .. }
        ));
        assert_eq!(
            *advisor.expires_at.lock().expect("deadline recording lock"),
            Some(deadline.expires_at())
        );
    }

    #[test]
    fn semantic_advisor_cannot_confirm_zero_after_the_resolution_deadline() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        env.owner_client.seed_repository_issue(owner_issue(
            7,
            IssueState::Closed,
            "unrelated public issue",
        ));
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_millis(200));
        let mut candidate = candidate(true);

        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &SlowSemanticAdvisor(Duration::from_millis(250)),
            &deadline,
        )
        .expect("deadline outcome");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::Blocked {
                reason: BlockedReason::Timeout,
                ..
            }
        ));
    }

    #[test]
    fn owner_preflight_maps_typed_failures_and_posts_taint_free_board_status() {
        let failures = vec![
            (
                GitHubApiError::PartialPage {
                    operation: "private-page".to_string(),
                    completed_pages: 1,
                },
                crate::cli::improvement::BlockedReason::Search,
                Some(crate::cli::improvement::FailureSubcode::PartialPage),
            ),
            (
                GitHubApiError::Unauthorized,
                crate::cli::improvement::BlockedReason::Auth,
                None,
            ),
            (
                GitHubApiError::Timeout {
                    operation: "private-timeout".to_string(),
                },
                crate::cli::improvement::BlockedReason::Timeout,
                None,
            ),
            (
                GitHubApiError::RateLimited {
                    retry_after: Some(60),
                },
                crate::cli::improvement::BlockedReason::RateLimit,
                None,
            ),
            (
                GitHubApiError::Network("private-host.example".to_string()),
                crate::cli::improvement::BlockedReason::Network,
                None,
            ),
            (
                GitHubApiError::Parse {
                    operation: "private-json".to_string(),
                    message: "secret payload".to_string(),
                },
                crate::cli::improvement::BlockedReason::Parse,
                None,
            ),
        ];
        for (error, expected_reason, expected_subcode) in failures {
            let project = tempfile::tempdir().expect("project");
            let mut env = TestEnv::new(project.path().join("cache"));
            env.repo_path = project.path().to_path_buf();
            env.owner_client.fail_next_owner_operation(
                OwnerRepositoryOperation::ListIssues,
                OwnerRepositoryFaultTiming::BeforeSubmit,
                error,
            );
            let mut candidate = candidate(true);
            let outcome = owner_resolution_preflight(
                &mut env,
                &mut candidate,
                &ContractRoutingRegistry::default(),
                &StaticSemanticAdvisor(Ok(Vec::new())),
                &resolution_deadline(),
            )
            .expect("preflight");
            assert!(matches!(
                outcome,
                OwnerPreflightOutcome::Blocked { reason, failure_subcode }
                    if reason == expected_reason && failure_subcode == expected_subcode
            ));
            assert_eq!(env.owner_client.owner_mutation_count(), 0);
            let bodies = board_bodies(&mut env);
            let body = bodies.last().expect("blocked board status");
            assert!(body.contains(candidate.id.as_str()));
            for taint in [
                "private-page",
                "private-timeout",
                "private-host",
                "secret payload",
            ] {
                assert!(!body.contains(taint), "board leaked {taint}: {body}");
            }
        }
    }

    #[test]
    fn revision_pinned_contract_mapping_is_authoritative() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        env.owner_client.seed_repository_issue(owner_issue(
            77,
            IssueState::Open,
            "owner selected by pinned contract",
        ));
        let mut candidate = candidate(true);
        let source_scope_nonce = "0".repeat(64);
        let source_event_id = "event-route-1";
        let opaque_key = opaque_occurrence_key(
            &source_scope_nonce,
            candidate.fingerprint.as_deref().expect("fingerprint"),
            "test.owner-route.v1",
            source_event_id,
        );
        candidate.distinct_occurrences.push(DistinctOccurrence {
            opaque_key,
            evidence_digest: "3f649bd386b953b42442e8cefcbd1449d657f49a972f11d72f810bcda167756a"
                .to_string(),
            captured_at: "2026-07-14T00:00:00Z".to_string(),
            origin: OccurrenceOrigin::Deterministic,
            qualifies_unattended: true,
            producer_id: Some("test.owner-route.v1".to_string()),
            producer_registry_revision: Some(1),
            routing_basis_revision: Some(9),
            replay_proof: Some(OccurrenceReplayProof::RegisteredEvent {
                source_scope_nonce,
                source_event_id: source_event_id.to_string(),
            }),
            recurrence: None,
        });
        candidate.occurrences = 2;
        let registry = ContractRoutingRegistry::new(vec![ContractRouteMapping {
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            routing_basis_revision: 9,
            disposition: ContractRouteDisposition::ExistingOwner(IssueNumber(77)),
        }]);
        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &registry,
            &StaticSemanticAdvisor(Ok(Vec::new())),
            &resolution_deadline(),
        )
        .expect("preflight");
        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::Active { owner, .. }
                if owner.number == 77 && owner.match_basis == OwnerMatchBasis::Contract
        ));
    }

    #[test]
    fn revision_pinned_contract_mapping_ignores_nonqualifying_occurrences() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        env.owner_client.seed_repository_issue(owner_issue(
            77,
            IssueState::Open,
            "unrelated public issue",
        ));
        let mut candidate = candidate(true);
        let source_scope_nonce = "0".repeat(64);
        let source_event_id = "event-route-1";
        let opaque_key = opaque_occurrence_key(
            &source_scope_nonce,
            candidate.fingerprint.as_deref().expect("fingerprint"),
            "test.owner-route.v1",
            source_event_id,
        );
        candidate.distinct_occurrences.push(DistinctOccurrence {
            opaque_key,
            evidence_digest: "3f649bd386b953b42442e8cefcbd1449d657f49a972f11d72f810bcda167756a"
                .to_string(),
            captured_at: "2026-07-14T00:00:00Z".to_string(),
            origin: OccurrenceOrigin::Deterministic,
            qualifies_unattended: false,
            producer_id: Some("test.owner-route.v1".to_string()),
            producer_registry_revision: Some(1),
            routing_basis_revision: Some(9),
            replay_proof: Some(OccurrenceReplayProof::RegisteredEvent {
                source_scope_nonce,
                source_event_id: source_event_id.to_string(),
            }),
            recurrence: None,
        });
        candidate.occurrences = 2;
        let registry = ContractRoutingRegistry::new(vec![ContractRouteMapping {
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            routing_basis_revision: 9,
            disposition: ContractRouteDisposition::ExistingOwner(IssueNumber(77)),
        }]);

        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &registry,
            &StaticSemanticAdvisor(Ok(Vec::new())),
            &resolution_deadline(),
        )
        .expect("preflight");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::Blocked {
                reason: BlockedReason::Routing,
                ..
            }
        ));
    }

    #[test]
    fn proven_zero_requires_revision_pinned_implementation_gap_route() {
        let cases = [
            (
                "implementation-gap",
                Some(ContractRouteDisposition::ImplementationGap),
                None,
            ),
            (
                "aligned",
                Some(ContractRouteDisposition::Aligned),
                Some(BlockedReason::Routing),
            ),
            (
                "spec-gap",
                Some(ContractRouteDisposition::SpecGap),
                Some(BlockedReason::Ambiguity),
            ),
            (
                "spec-ambiguous",
                Some(ContractRouteDisposition::SpecAmbiguous),
                Some(BlockedReason::Ambiguity),
            ),
            ("missing", None, Some(BlockedReason::Routing)),
        ];

        for (name, disposition, blocked_reason) in cases {
            let project = tempfile::tempdir().expect("project");
            let mut env = TestEnv::new(project.path().join("cache"));
            env.repo_path = project.path().to_path_buf();
            env.owner_client.seed_repository_issue(owner_issue(
                77,
                IssueState::Open,
                "unrelated public issue",
            ));
            let mut candidate = candidate(true);
            let registry = ContractRoutingRegistry::new(
                disposition
                    .map(|disposition| ContractRouteMapping {
                        contract_id: "coordination.board-status".to_string(),
                        contract_schema_revision: 1,
                        failure_code: "STATUS_NOT_POSTED".to_string(),
                        expected_outcome: "BOARD_STATUS_POSTED".to_string(),
                        routing_basis_revision: 1,
                        disposition,
                    })
                    .into_iter()
                    .collect(),
            );

            let outcome = owner_resolution_preflight(
                &mut env,
                &mut candidate,
                &registry,
                &StaticSemanticAdvisor(Ok(Vec::new())),
                &resolution_deadline(),
            )
            .unwrap_or_else(|error| panic!("{name}: preflight failed: {error}"));

            match blocked_reason {
                None => assert!(
                    matches!(outcome, OwnerPreflightOutcome::CreateAuthorized { .. }),
                    "{name}: {outcome:?}"
                ),
                Some(reason) => assert!(
                    matches!(
                        outcome,
                        OwnerPreflightOutcome::Blocked {
                            reason: actual,
                            ..
                        } if actual == reason
                    ),
                    "{name}: {outcome:?}"
                ),
            }
            assert_eq!(env.owner_client.owner_mutation_count(), 0, "{name}");
        }

        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().to_path_buf();
        env.owner_client.seed_repository_issue(owner_issue(
            77,
            IssueState::Open,
            "unrelated public issue",
        ));
        let mut candidate = candidate(true);
        let stale_registry = ContractRoutingRegistry::new(vec![ContractRouteMapping {
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            routing_basis_revision: 2,
            disposition: ContractRouteDisposition::ImplementationGap,
        }]);
        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &stale_registry,
            &StaticSemanticAdvisor(Ok(Vec::new())),
            &resolution_deadline(),
        )
        .expect("stale route preflight");
        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::Blocked {
                reason: BlockedReason::Routing,
                ..
            }
        ));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
    }

    #[test]
    fn complete_zero_with_current_implementation_gap_creates_one_plain_owner_and_projection() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let candidate_id = "impr-proven-zero-create";
        let (fingerprint, occurrence_key) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "proven-zero-event");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            77,
            IssueState::Open,
            "unrelated public issue",
        ));
        let mut candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source store")
            .candidates
            .remove(0);
        let deadline = resolution_deadline();
        let expected_payload = render_public_issue_payload(
            &candidate,
            &PublicMutationContext::for_repo_with_deadline(source.path(), &deadline),
        )
        .expect("typed public payload");

        let preflight = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &implementation_gap_registry(1),
            &NoSemanticOwnerAdvisor,
            &deadline,
        )
        .expect("proven-zero preflight");
        let OwnerPreflightOutcome::CreateAuthorized { authorization } = preflight else {
            panic!("expected proven-zero create authorization");
        };

        commit_new_owner_resolution(&mut env, &mut candidate, &authorization, &deadline)
            .expect("new owner commit");

        assert_eq!(candidate.state, CandidateState::Created);
        assert_eq!(candidate.owner.as_ref().unwrap().number, 78);
        assert_eq!(candidate.owner.as_ref().unwrap().kind, OwnerKind::Issue);
        assert_eq!(candidate.owner.as_ref().unwrap().fingerprint, fingerprint);
        assert_eq!(candidate.linked_issue.as_ref().unwrap().number, 78);
        assert_eq!(candidate.blocked_reason, None);
        assert_eq!(candidate.retry, None);

        let mutations = env.owner_client.owner_mutation_call_log();
        assert_eq!(mutations.len(), 1);
        assert_eq!(
            mutations[0].operation,
            OwnerRepositoryOperation::CreateIssue
        );
        assert_eq!(mutations[0].repository, RepositoryIdentity::gwt_upstream());
        assert_eq!(mutations[0].issue_number, Some(IssueNumber(78)));
        let readback = env
            .owner_client
            .fetch_issue(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(78),
                &deadline,
            )
            .expect("created owner readback");
        assert_eq!(readback.title, expected_payload.title);
        assert_eq!(readback.body, expected_payload.body);
        assert!(readback.labels.is_empty());
        assert_eq!(readback.kind, RepositoryIssueKind::Plain);
        assert_eq!(readback.state, IssueState::Open);
        assert!(exact_fingerprint_markers(&readback.body)
            .iter()
            .any(|marker| marker == &fingerprint));

        let source_candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("repaired source store")
            .candidates
            .remove(0);
        assert_eq!(source_candidate.state, CandidateState::Created);
        assert_eq!(source_candidate.owner.as_ref().unwrap().number, 78);
        let projection =
            crate::cli::improvement_contract::read_owner_projection().expect("owner projection");
        assert_eq!(projection.owners.len(), 1);
        assert_eq!(projection.owners[0].owner.number, 78);
        assert_eq!(projection.owners[0].aggregate_count, 1);
        assert_eq!(
            projection.owners[0].occurrences[0].opaque_key,
            occurrence_key
        );
        assert!(board_bodies(&mut env)
            .last()
            .is_some_and(|body| { body.contains("was created") && body.contains("#78") }));
    }

    fn store_projection_candidate(
        repo_root: &Path,
        nonce: &str,
        candidate_id: &str,
        source_event_id: &str,
    ) -> (String, String) {
        let mut candidate = candidate(true);
        candidate.id = candidate_id.to_string();
        candidate.sanitized_summary = format!(
            "Private candidate from {}",
            repo_root.join("customer/private-target").display()
        );
        let fingerprint = candidate.fingerprint.clone().expect("fingerprint");
        let occurrence_key = opaque_occurrence_key(
            nonce,
            &fingerprint,
            "test.coordination-gate.v1",
            source_event_id,
        );
        let occurrence = candidate
            .distinct_occurrences
            .first_mut()
            .expect("typed occurrence");
        occurrence.opaque_key = occurrence_key.clone();
        occurrence.replay_proof = Some(OccurrenceReplayProof::RegisteredEvent {
            source_scope_nonce: nonce.to_string(),
            source_event_id: source_event_id.to_string(),
        });
        candidate.occurrences = 1;
        crate::cli::improvement_store::update(repo_root, |store| {
            store.source_scope_nonce = Some(nonce.to_string());
            store.candidates = vec![candidate];
            Ok(())
        })
        .expect("store source candidate");
        (fingerprint, occurrence_key)
    }

    fn replace_candidate_public_outcomes(repo_root: &Path, expected: &str, observed: &str) {
        crate::cli::improvement_store::update(repo_root, |store| {
            let candidate = store.candidates.first_mut().expect("source candidate");
            let evidence = candidate.typed_evidence.as_mut().expect("typed evidence");
            evidence.expected_outcome = expected.to_string();
            evidence.observed_outcome = observed.to_string();
            let evidence = evidence.clone();
            candidate.evidence_digest = Some(typed_evidence_digest(&evidence));
            for occurrence in &mut candidate.distinct_occurrences {
                occurrence.evidence_digest =
                    occurrence_evidence_digest(&evidence, occurrence.recurrence.as_ref());
            }
            Ok(())
        })
        .expect("replace current public evidence");
    }

    fn verified_projection_owner(number: u64, fingerprint: &str) -> DurableOwnerSnapshot {
        DurableOwnerSnapshot {
            number,
            kind: OwnerKind::Issue,
            title: format!("Verified public owner {number}"),
            active: true,
            url: format!("https://github.com/akiojin/gwt/issues/{number}"),
            fingerprint: fingerprint.to_string(),
            readback_verified_at: "2026-07-15T08:00:00Z".to_string(),
        }
    }

    fn append_projection_occurrence(
        repo_root: &Path,
        nonce: &str,
        source_event_id: &str,
    ) -> String {
        let store = crate::cli::improvement_store::load_and_repair(repo_root)
            .expect("load source candidate");
        let candidate = store.candidates.first().expect("source candidate");
        let fingerprint = candidate.fingerprint.as_deref().expect("fingerprint");
        let producer_id = candidate.distinct_occurrences[0]
            .producer_id
            .as_deref()
            .expect("registered producer");
        let occurrence_key =
            opaque_occurrence_key(nonce, fingerprint, producer_id, source_event_id);
        crate::cli::improvement_store::update(repo_root, |store| {
            let candidate = store.candidates.first_mut().expect("source candidate");
            let mut occurrence = candidate.distinct_occurrences[0].clone();
            occurrence.opaque_key = occurrence_key.clone();
            occurrence.captured_at = "2026-07-15T09:00:00Z".to_string();
            occurrence.replay_proof = Some(OccurrenceReplayProof::RegisteredEvent {
                source_scope_nonce: nonce.to_string(),
                source_event_id: source_event_id.to_string(),
            });
            candidate.distinct_occurrences.push(occurrence);
            candidate.occurrences = candidate.distinct_occurrences.len() as u64;
            candidate.updated_at = "2026-07-15T09:00:00Z".to_string();
            Ok(())
        })
        .expect("append source occurrence");
        occurrence_key
    }

    #[test]
    fn active_issue_and_spec_resolution_create_one_missing_comment_then_commit_projection_first() {
        for (owner_number, owner_kind) in [
            (42, RepositoryIssueKind::Plain),
            (43, RepositoryIssueKind::Spec),
        ] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let source = tempfile::tempdir().expect("source");
            let nonce = "0".repeat(64);
            let candidate_id = format!("impr-active-owner-{owner_number}");
            let (fingerprint, occurrence_key) = store_projection_candidate(
                source.path(),
                &nonce,
                &candidate_id,
                &format!("active-owner-event-{owner_number}"),
            );
            let mut env = TestEnv::new(source.path().join("cache"));
            env.repo_path = source.path().to_path_buf();
            env.improvement_source_scope_nonce = nonce;
            let mut issue = owner_issue(
                owner_number,
                IssueState::Open,
                fingerprint_marker(&fingerprint),
            );
            issue.kind = owner_kind;
            if owner_kind == RepositoryIssueKind::Spec {
                issue.labels = vec!["gwt-spec".to_string()];
            }
            let original_body = issue.body.clone();
            env.owner_client.seed_repository_issue(issue);
            let mut candidate = crate::cli::improvement_store::load_and_repair(source.path())
                .expect("source store")
                .candidates
                .remove(0);
            let deadline = resolution_deadline();

            let preflight = owner_resolution_preflight(
                &mut env,
                &mut candidate,
                &ContractRoutingRegistry::default(),
                &NoSemanticOwnerAdvisor,
                &deadline,
            )
            .expect("active owner preflight");
            let OwnerPreflightOutcome::Active {
                owner, comments, ..
            } = preflight
            else {
                panic!("expected active owner preflight");
            };
            assert!(comments.is_empty());

            commit_active_owner_resolution(&mut env, &mut candidate, &owner, &comments, &deadline)
                .expect("active owner commit");

            assert_eq!(candidate.state, CandidateState::Linked);
            assert_eq!(candidate.owner.as_ref().unwrap().number, owner_number);
            assert_eq!(
                candidate.owner.as_ref().unwrap().kind,
                match owner_kind {
                    RepositoryIssueKind::Plain => OwnerKind::Issue,
                    RepositoryIssueKind::Spec => OwnerKind::Spec,
                }
            );
            let source_candidate = crate::cli::improvement_store::load_and_repair(source.path())
                .expect("repaired source store")
                .candidates
                .remove(0);
            assert_eq!(source_candidate.state, CandidateState::Linked);
            assert_eq!(
                source_candidate.owner.as_ref().unwrap().number,
                owner_number
            );

            let projection = crate::cli::improvement_contract::read_owner_projection()
                .expect("owner projection");
            assert_eq!(projection.owners.len(), 1);
            assert_eq!(projection.owners[0].owner.number, owner_number);
            assert_eq!(projection.owners[0].aggregate_count, 1);
            assert_eq!(
                projection.owners[0].occurrences[0].opaque_key,
                occurrence_key
            );

            let mutations = env.owner_client.owner_mutation_call_log();
            assert_eq!(mutations.len(), 1);
            assert_eq!(
                mutations[0].operation,
                OwnerRepositoryOperation::CreateComment
            );
            assert_eq!(mutations[0].issue_number, Some(IssueNumber(owner_number)));
            let comments = env
                .owner_client
                .list_comments(
                    &RepositoryIdentity::gwt_upstream(),
                    IssueNumber(owner_number),
                    &deadline,
                )
                .expect("comment readback");
            assert_eq!(comments.items().len(), 1);
            assert!(comments.items()[0]
                .body
                .lines()
                .any(|line| line == occurrence_marker(&occurrence_key)));
            let readback = env
                .owner_client
                .fetch_issue(
                    &RepositoryIdentity::gwt_upstream(),
                    IssueNumber(owner_number),
                    &deadline,
                )
                .expect("owner readback");
            assert_eq!(readback.body, original_body);
            assert_eq!(readback.state, IssueState::Open);
            assert_eq!(readback.kind, owner_kind);
            assert!(board_bodies(&mut env).last().is_some_and(|body| {
                body.contains("was linked") && body.contains(&format!("#{owner_number}"))
            }));

            let retry_preflight = owner_resolution_preflight(
                &mut env,
                &mut candidate,
                &ContractRoutingRegistry::default(),
                &NoSemanticOwnerAdvisor,
                &deadline,
            )
            .expect("retry owner preflight");
            let OwnerPreflightOutcome::Active {
                owner, comments, ..
            } = retry_preflight
            else {
                panic!("expected active owner on retry");
            };
            let retry_error = commit_active_owner_resolution(
                &mut env,
                &mut candidate,
                &owner,
                &comments,
                &deadline,
            )
            .expect_err("successful candidates require a new OwnerResolving lease");
            assert!(matches!(
                retry_error,
                OwnerResolutionCommitError::PreSubmit {
                    failure: OwnerResolutionFailure {
                        reason: BlockedReason::Store,
                        ..
                    },
                    ..
                }
            ));
            assert_eq!(env.owner_client.owner_mutation_count(), 1);
            assert_eq!(
                env.owner_client
                    .list_comments(
                        &RepositoryIdentity::gwt_upstream(),
                        IssueNumber(owner_number),
                        &deadline,
                    )
                    .expect("retry comment readback")
                    .items()
                    .len(),
                1
            );
        }
    }

    #[test]
    fn active_owner_resolution_dedupes_simultaneous_physical_occurrence_comments_logically() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let candidate_id = "impr-active-owner-duplicate-comments";
        let (fingerprint, occurrence_key) = store_projection_candidate(
            source.path(),
            &nonce,
            candidate_id,
            "duplicate-comment-event",
        );
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            44,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        let mut candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source store")
            .candidates
            .remove(0);
        let public_context = PublicMutationContext::for_repo(source.path());
        let comment_body =
            render_occurrence_comment_payload(&candidate, &occurrence_key, &public_context)
                .expect("occurrence payload")
                .body;
        env.owner_client.seed_repository_comments(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(44),
            vec![
                repository_comment(1, comment_body.clone(), "2026-07-15T10:00:00Z"),
                repository_comment(2, comment_body, "2026-07-15T10:00:01Z"),
            ],
        );
        let deadline = resolution_deadline();

        let preflight = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &deadline,
        )
        .expect("active owner preflight");
        let OwnerPreflightOutcome::Active {
            owner, comments, ..
        } = preflight
        else {
            panic!("expected active owner preflight");
        };
        assert_eq!(comments.len(), 2);

        commit_active_owner_resolution(&mut env, &mut candidate, &owner, &comments, &deadline)
            .expect("adopt duplicate physical comments");

        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        assert_eq!(candidate.state, CandidateState::Linked);
        let projection =
            crate::cli::improvement_contract::read_owner_projection().expect("owner projection");
        assert_eq!(projection.owners.len(), 1);
        assert_eq!(projection.owners[0].aggregate_count, 1);
        assert_eq!(
            projection.owners[0].occurrences[0].opaque_key,
            occurrence_key
        );
        let public_projection =
            serde_json::to_value(projection).expect("serialize public projection");
        assert!(!public_projection.to_string().contains("comment_audit"));
        assert!(!public_projection.to_string().contains("comment_id"));
        let private_projection: serde_json::Value = serde_json::from_slice(
            &fs::read(crate::cli::improvement_store::owner_projection_path())
                .expect("private projection storage"),
        )
        .expect("parse private projection storage");
        assert_eq!(
            private_projection["owners"][0]["occurrences"][0]["comment_audit"],
            json!({
                "completeness": "complete",
                "physical_comments": [
                    {"owner_number": 44, "comment_id": 1},
                    {"owner_number": 44, "comment_id": 2}
                ]
            })
        );
    }

    #[test]
    fn active_owner_resolution_projects_qualifying_and_prior_nonqualifying_occurrences() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let candidate_id = "impr-active-owner-complete-history";
        let (fingerprint, first_occurrence_key) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "historical-event");
        let second_occurrence_key =
            append_projection_occurrence(source.path(), &nonce, "qualifying-event");
        crate::cli::improvement_store::update(source.path(), |store| {
            store.candidates[0].distinct_occurrences[0].qualifies_unattended = false;
            Ok(())
        })
        .expect("mark prior occurrence nonqualifying");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            46,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        let mut candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source store")
            .candidates
            .remove(0);
        let deadline = resolution_deadline();

        let preflight = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &deadline,
        )
        .expect("active owner preflight");
        let OwnerPreflightOutcome::Active {
            owner, comments, ..
        } = preflight
        else {
            panic!("expected active owner preflight");
        };
        commit_active_owner_resolution(&mut env, &mut candidate, &owner, &comments, &deadline)
            .expect("active owner commit");

        let projection =
            crate::cli::improvement_contract::read_owner_projection().expect("owner projection");
        assert_eq!(projection.owners.len(), 1);
        assert_eq!(projection.owners[0].aggregate_count, 2);
        assert_eq!(
            projection.owners[0]
                .occurrences
                .iter()
                .map(|occurrence| occurrence.opaque_key.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                first_occurrence_key.as_str(),
                second_occurrence_key.as_str(),
            ])
        );
        assert_eq!(env.owner_client.owner_mutation_count(), 2);
        assert_eq!(candidate.state, CandidateState::Linked);
        assert_eq!(candidate.occurrences, 2);
    }

    #[test]
    fn active_owner_resolution_rejects_marker_lookalikes_and_creates_canonical_comment() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let candidate_id = "impr-active-owner-marker-lookalikes";
        let (fingerprint, occurrence_key) = store_projection_candidate(
            source.path(),
            &nonce,
            candidate_id,
            "marker-lookalike-event",
        );
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            47,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        let exact_occurrence = occurrence_marker(&occurrence_key);
        let exact_fingerprint = fingerprint_marker(&fingerprint);
        env.owner_client.seed_repository_comments(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(47),
            vec![
                repository_comment(
                    1,
                    format!("{exact_occurrence} suffix\n{exact_fingerprint}"),
                    "2026-07-15T10:00:00Z",
                ),
                repository_comment(
                    2,
                    format!("```\n{exact_occurrence}\n{exact_fingerprint}\n```"),
                    "2026-07-15T10:00:01Z",
                ),
                repository_comment(
                    3,
                    format!(
                        "{exact_occurrence}\n{}",
                        fingerprint_marker(&format!("v2:{}", "f".repeat(64)))
                    ),
                    "2026-07-15T10:00:02Z",
                ),
            ],
        );
        let mut candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source store")
            .candidates
            .remove(0);
        let deadline = resolution_deadline();

        let preflight = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &deadline,
        )
        .expect("active owner preflight");
        let OwnerPreflightOutcome::Active {
            owner, comments, ..
        } = preflight
        else {
            panic!("expected active owner preflight");
        };
        commit_active_owner_resolution(&mut env, &mut candidate, &owner, &comments, &deadline)
            .expect("active owner commit");

        assert_eq!(env.owner_client.owner_mutation_count(), 1);
        let comments = env
            .owner_client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(47),
                &deadline,
            )
            .expect("comment readback");
        assert_eq!(comments.items().len(), 4);
        assert_eq!(
            comments
                .items()
                .iter()
                .filter(|comment| {
                    comment_matches_occurrence(comment, &occurrence_key, &fingerprint)
                })
                .count(),
            1
        );
    }

    #[test]
    fn active_owner_title_privacy_failure_happens_before_comment_mutation() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let (fingerprint, _) = store_projection_candidate(
            source.path(),
            &nonce,
            "impr-active-owner-private-title",
            "private-title-event",
        );
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        let mut issue = owner_issue(48, IssueState::Open, fingerprint_marker(&fingerprint));
        issue.title = format!("Owner from {}", source.path().display());
        env.owner_client.seed_repository_issue(issue);
        let mut candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source store")
            .candidates
            .remove(0);
        let deadline = resolution_deadline();

        let preflight = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &deadline,
        )
        .expect("active owner preflight");
        let OwnerPreflightOutcome::Active {
            owner, comments, ..
        } = preflight
        else {
            panic!("expected active owner preflight");
        };
        let error =
            commit_active_owner_resolution(&mut env, &mut candidate, &owner, &comments, &deadline)
                .expect_err("private owner title must fail closed");

        assert!(error.to_string().contains("privacy validation failed"));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        assert!(!crate::cli::improvement_store::owner_projection_path().exists());
        assert_eq!(candidate.state, CandidateState::OwnerResolving);
    }

    #[test]
    fn one_active_exact_owner_wins_when_historical_exact_owners_also_exist() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let (fingerprint, _) = store_projection_candidate(
            source.path(),
            &nonce,
            "impr-active-with-history",
            "active-with-history-event",
        );
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            49,
            IssueState::Closed,
            fingerprint_marker(&fingerprint),
        ));
        env.owner_client.seed_repository_issue(owner_issue(
            50,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        let mut candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source store")
            .candidates
            .remove(0);
        let deadline = resolution_deadline();

        let preflight = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &deadline,
        )
        .expect("owner preflight");
        let OwnerPreflightOutcome::Active {
            owner, comments, ..
        } = preflight
        else {
            panic!("one active exact owner must win over historical owners");
        };
        assert_eq!(owner.number, 50);
        commit_active_owner_resolution(&mut env, &mut candidate, &owner, &comments, &deadline)
            .expect("active owner commit");

        assert_eq!(candidate.state, CandidateState::Linked);
        assert_eq!(candidate.owner.as_ref().unwrap().number, 50);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
        assert!(env
            .owner_client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(49),
                &deadline,
            )
            .expect("historical owner comments")
            .items()
            .is_empty());
    }

    #[test]
    fn source_growth_after_preflight_cannot_report_false_linked_success() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let (fingerprint, _) = store_projection_candidate(
            source.path(),
            &nonce,
            "impr-active-owner-source-growth",
            "source-growth-first-event",
        );
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce.clone();
        env.owner_client.seed_repository_issue(owner_issue(
            51,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        let mut candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source store")
            .candidates
            .remove(0);
        let deadline = resolution_deadline();
        let preflight = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &deadline,
        )
        .expect("active owner preflight");
        let OwnerPreflightOutcome::Active {
            owner, comments, ..
        } = preflight
        else {
            panic!("expected active owner preflight");
        };
        append_projection_occurrence(source.path(), &nonce, "source-growth-second-event");

        let error =
            commit_active_owner_resolution(&mut env, &mut candidate, &owner, &comments, &deadline)
                .expect_err("incomplete projection coverage must not report linked");

        assert!(error.to_string().contains("projection coverage"));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        assert!(!crate::cli::improvement_store::owner_projection_path().exists());
        assert_eq!(candidate.state, CandidateState::OwnerResolving);
        assert_eq!(candidate.occurrences, 1);
        let source_candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source store")
            .candidates
            .remove(0);
        assert_eq!(source_candidate.state, CandidateState::OwnerResolving);
        assert_eq!(source_candidate.occurrences, 2);
        assert!(!board_bodies(&mut env)
            .iter()
            .any(|body| body.contains("was linked")));
    }

    #[test]
    fn invalid_later_projection_input_fails_before_any_comment_mutation() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let (fingerprint, _) = store_projection_candidate(
            source.path(),
            &nonce,
            "impr-active-owner-invalid-projection-input",
            "valid-first-event",
        );
        append_projection_occurrence(source.path(), &nonce, "invalid-second-event");
        crate::cli::improvement_store::update(source.path(), |store| {
            store.candidates[0].distinct_occurrences[1].captured_at =
                "not-an-rfc3339-timestamp".to_string();
            Ok(())
        })
        .expect("seed invalid later projection input");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            52,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        let mut candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source store")
            .candidates
            .remove(0);
        let deadline = resolution_deadline();
        let preflight = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &deadline,
        )
        .expect("active owner preflight");
        let OwnerPreflightOutcome::Active {
            owner, comments, ..
        } = preflight
        else {
            panic!("expected active owner preflight");
        };

        let error =
            commit_active_owner_resolution(&mut env, &mut candidate, &owner, &comments, &deadline)
                .expect_err("invalid projection input must fail closed");

        assert!(error.to_string().contains("last_seen must be RFC3339"));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        assert!(!crate::cli::improvement_store::owner_projection_path().exists());
    }

    #[test]
    fn closed_exact_owner_remains_historical_without_comment_or_projection_mutation() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let (fingerprint, _) = store_projection_candidate(
            source.path(),
            &nonce,
            "impr-historical-owner",
            "historical-owner-event",
        );
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Closed,
            fingerprint_marker(&fingerprint),
        ));
        let mut candidate = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("source store")
            .candidates
            .remove(0);

        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &resolution_deadline(),
        )
        .expect("historical owner preflight");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::Historical { owners, .. }
                if owners.len() == 1 && owners[0].number == 45 && !owners[0].active
        ));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        assert!(!crate::cli::improvement_store::owner_projection_path().exists());
        let readback = env
            .owner_client
            .fetch_issue(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(45),
                &resolution_deadline(),
            )
            .expect("historical owner readback");
        assert_eq!(readback.state, IssueState::Closed);
    }

    #[test]
    fn verified_historical_owner_with_newer_version_creates_one_regression_issue() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let (fingerprint, _) = store_projection_candidate(
            source.path(),
            &nonce,
            "impr-historical-regression",
            "historical-regression-event",
        );
        crate::cli::improvement_store::update(source.path(), |store| {
            let mut recurrent = store.candidates[0].clone();
            let recurrence = TypedRecurrenceEvidence {
                installed_version: Some("9.66.0".to_string()),
                build_commit: None,
                observed_at: "2026-07-15T09:00:00Z".to_string(),
            };
            let evidence_digest = occurrence_evidence_digest(
                recurrent.typed_evidence.as_ref().expect("typed evidence"),
                Some(&recurrence),
            );
            recurrent.distinct_occurrences[0].captured_at = "2026-07-16T00:00:00Z".to_string();
            recurrent.distinct_occurrences[0].recurrence = Some(recurrence);
            recurrent.distinct_occurrences[0].evidence_digest = evidence_digest;
            recurrent.state = CandidateState::Recurrent;
            recurrent.owner = Some(DurableOwnerSnapshot {
                number: 45,
                kind: OwnerKind::Issue,
                title: "Owner 45".to_string(),
                active: true,
                url: "https://github.com/akiojin/gwt/issues/45".to_string(),
                fingerprint: fingerprint.clone(),
                readback_verified_at: "2026-07-10T00:00:00Z".to_string(),
            });
            store.candidates[0] = recurrent;
            Ok(())
        })
        .expect("store recurrent candidate");

        let repository = RepositoryIdentity::gwt_upstream();
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Closed,
            fingerprint_marker(&fingerprint),
        ));
        env.owner_client.seed_repository_comments(
            &repository,
            IssueNumber(45),
            vec![repository_comment(
                501,
                historical_resolution_comment(
                    900,
                    MERGE_COMMIT_SHA,
                    "2026-07-10T00:00:00Z",
                    "v9.65.0",
                ),
                "2026-07-10T00:00:00Z",
            )],
        );
        env.owner_client.seed_merged_pull_request(
            &repository,
            MergedPullRequest {
                number: IssueNumber(900),
                merge_commit_sha: MERGE_COMMIT_SHA.to_string(),
                merged_at: "2026-07-01T00:00:00Z".to_string(),
            },
        );
        env.owner_client.seed_repository_release(
            &repository,
            RepositoryRelease {
                tag_name: "v9.65.0".to_string(),
                target_commitish: "develop".to_string(),
                published_at: "2026-07-05T00:00:00Z".to_string(),
            },
        );
        env.owner_client.seed_commit_comparison(
            &repository,
            CommitComparison {
                base: MERGE_COMMIT_SHA.to_string(),
                head: "refs/tags/v9.65.0".to_string(),
                base_commit_sha: MERGE_COMMIT_SHA.to_string(),
                merge_base_commit_sha: MERGE_COMMIT_SHA.to_string(),
                head_commit_sha: RELEASE_COMMIT_SHA.to_string(),
                status: CommitComparisonStatus::Ahead,
                ahead_by: 4,
                behind_by: 0,
            },
        );

        let resolved = resolve_candidate_owner(
            &mut env,
            "impr-historical-regression",
            CaptureBudgetProfile::Normal,
        )
        .expect("verified recurrence resolution");

        assert_eq!(
            resolved.state,
            CandidateState::Created,
            "{resolved:?}; calls={:?}",
            env.owner_client.owner_call_log()
        );
        assert_eq!(resolved.owner.as_ref().map(|owner| owner.number), Some(46));
        let regression = env
            .owner_client
            .fetch_issue(&repository, IssueNumber(46), &resolution_deadline())
            .expect("regression owner readback");
        assert!(regression.body.contains("Historical owner: #45"));
        assert!(exact_fingerprint_markers(&regression.body)
            .iter()
            .any(|marker| marker == &fingerprint));
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(45), &resolution_deadline())
                .expect("historical owner readback")
                .state,
            IssueState::Closed
        );
        assert!(env
            .owner_client
            .owner_mutation_call_log()
            .iter()
            .all(|call| { call.issue_number != Some(IssueNumber(45)) }));
    }

    #[test]
    fn historical_recurrence_rejects_untrusted_or_conflicting_resolution_markers() {
        for (name, comments) in [
            ("bot", {
                let mut comment = standard_historical_resolution_comment();
                comment.author_type = Some(RepositoryActorType::Bot);
                vec![comment]
            }),
            ("untrusted-association", {
                let mut comment = standard_historical_resolution_comment();
                comment.author_association = Some(RepositoryAuthorAssociation::Contributor);
                vec![comment]
            }),
            ("missing-association", {
                let mut comment = standard_historical_resolution_comment();
                comment.author_association = None;
                vec![comment]
            }),
            ("unknown-association", {
                let mut comment = standard_historical_resolution_comment();
                comment.author_association = Some(RepositoryAuthorAssociation::Unknown(
                    "FUTURE_ROLE".to_string(),
                ));
                vec![comment]
            }),
            (
                "multiple-markers",
                vec![
                    standard_historical_resolution_comment(),
                    repository_comment(
                        502,
                        historical_resolution_comment(
                            901,
                            "other-merge",
                            "2026-07-11T00:00:00Z",
                            "v9.65.1",
                        ),
                        "2026-07-11T00:00:00Z",
                    ),
                ],
            ),
            (
                "malformed-marker",
                vec![repository_comment(
                    501,
                    "<!-- gwt-improvement-resolution:v1 not-json -->",
                    "2026-07-10T00:00:00Z",
                )],
            ),
        ] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let source = tempfile::tempdir().expect("source");
            let candidate_id = format!("impr-historical-{name}");
            let (_, mut env) = historical_regression_fixture(
                source.path(),
                &candidate_id,
                version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
                comments,
            );

            let resolved =
                resolve_candidate_owner(&mut env, &candidate_id, CaptureBudgetProfile::Normal)
                    .unwrap_or_else(|error| panic!("{name}: resolution failed: {error}"));

            assert_eq!(resolved.state, CandidateState::Blocked, "{name}");
            assert_eq!(
                resolved.blocked_reason,
                Some(BlockedReason::Ambiguity),
                "{name}"
            );
            assert_eq!(env.owner_client.owner_mutation_count(), 0, "{name}");
        }
    }

    #[test]
    fn historical_recurrence_rejects_untrusted_markers_even_with_a_trusted_marker() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-ignore-untrusted-markers";
        let mut bot = repository_comment(
            502,
            "<!-- gwt-improvement-resolution:v1 not-json -->",
            "2026-07-10T00:00:00Z",
        );
        bot.author_type = Some(RepositoryActorType::Bot);
        bot.author_association = Some(RepositoryAuthorAssociation::Owner);
        let mut contributor = repository_comment(
            503,
            historical_resolution_comment(
                901,
                CONFLICTING_COMMIT_SHA,
                "2026-07-11T00:00:00Z",
                "v9.65.1",
            ),
            "2026-07-11T00:00:00Z",
        );
        contributor.author_association = Some(RepositoryAuthorAssociation::Contributor);
        let mut missing_actor = repository_comment(
            504,
            "quoted gwt-improvement-resolution:v1 text",
            "2026-07-11T00:00:00Z",
        );
        missing_actor.author_login = None;
        missing_actor.author_type = None;
        missing_actor.author_association = None;
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![
                bot,
                contributor,
                missing_actor,
                standard_historical_resolution_comment(),
            ],
        );

        let resolved =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("untrusted marker evidence must fail closed");

        assert_eq!(resolved.state, CandidateState::Blocked);
        assert_eq!(resolved.blocked_reason, Some(BlockedReason::Ambiguity));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
    }

    #[test]
    fn enterprise_member_can_author_the_unique_historical_resolution_marker() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-enterprise-member";
        let mut comment = standard_historical_resolution_comment();
        comment.author_type = Some(RepositoryActorType::EnterpriseUserAccount);
        comment.author_association = Some(RepositoryAuthorAssociation::Member);
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![comment],
        );

        let resolved =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("trusted enterprise member marker");

        assert_eq!(resolved.state, CandidateState::Created);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn historical_recurrence_rejects_missing_stale_or_contradictory_version_proof() {
        for (name, recurrence) in [
            ("missing", version_recurrence(None, "2026-07-15T09:00:00Z")),
            (
                "invalid",
                version_recurrence(Some("not-semver"), "2026-07-15T09:00:00Z"),
            ),
            (
                "equal",
                version_recurrence(Some("9.65.0"), "2026-07-15T09:00:00Z"),
            ),
            (
                "older",
                version_recurrence(Some("9.64.9"), "2026-07-15T09:00:00Z"),
            ),
            (
                "pre-resolution",
                version_recurrence(Some("9.66.0"), "2026-07-10T00:00:00Z"),
            ),
            (
                "invalid-time",
                version_recurrence(Some("9.66.0"), "not-rfc3339"),
            ),
            (
                "future-observed",
                version_recurrence(Some("9.66.0"), "2999-07-15T09:00:00Z"),
            ),
        ] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let source = tempfile::tempdir().expect("source");
            let candidate_id = format!("impr-historical-proof-{name}");
            let (_, mut env) = historical_regression_fixture(
                source.path(),
                &candidate_id,
                recurrence,
                vec![standard_historical_resolution_comment()],
            );

            let resolved =
                resolve_candidate_owner(&mut env, &candidate_id, CaptureBudgetProfile::Normal)
                    .unwrap_or_else(|error| panic!("{name}: resolution failed: {error}"));

            assert_eq!(resolved.state, CandidateState::Blocked, "{name}");
            assert_eq!(
                resolved.blocked_reason,
                Some(BlockedReason::Ambiguity),
                "{name}"
            );
            assert_eq!(env.owner_client.owner_mutation_count(), 0, "{name}");
        }
    }

    #[test]
    fn historical_recurrence_rejects_nonqualifying_recurrence_with_prior_qualifying_evidence() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-nonqualifying-recurrence";
        let (fingerprint, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        crate::cli::improvement_store::update(source.path(), |store| {
            let candidate = &mut store.candidates[0];
            candidate.distinct_occurrences[0].qualifies_unattended = false;
            let mut prior = candidate.distinct_occurrences[0].clone();
            prior.opaque_key = opaque_occurrence_key(
                &"0".repeat(64),
                &fingerprint,
                "test.coordination-gate.v1",
                "prior-qualifying-evidence",
            );
            prior.evidence_digest = occurrence_evidence_digest(
                candidate.typed_evidence.as_ref().expect("typed evidence"),
                None,
            );
            prior.captured_at = "2026-07-09T00:00:00Z".to_string();
            prior.qualifies_unattended = true;
            prior.recurrence = None;
            prior.replay_proof = Some(OccurrenceReplayProof::RegisteredEvent {
                source_scope_nonce: "0".repeat(64),
                source_event_id: "prior-qualifying-evidence".to_string(),
            });
            candidate.distinct_occurrences.push(prior);
            candidate.occurrences = 2;
            Ok(())
        })
        .expect("append prior qualifying occurrence");

        let resolved =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("nonqualifying recurrence is persisted as blocked");

        assert_eq!(resolved.state, CandidateState::Blocked);
        assert_eq!(resolved.blocked_reason, Some(BlockedReason::Ambiguity));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
    }

    #[test]
    fn verified_historical_owner_accepts_a_strict_descendant_build_commit() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-descendant-build";
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            TypedRecurrenceEvidence {
                installed_version: None,
                build_commit: Some(BUILD_COMMIT_SHA.to_string()),
                observed_at: "2026-07-15T09:00:00Z".to_string(),
            },
            vec![standard_historical_resolution_comment()],
        );
        env.owner_client.seed_commit_comparison(
            &RepositoryIdentity::gwt_upstream(),
            CommitComparison {
                base: RELEASE_COMMIT_SHA.to_string(),
                head: BUILD_COMMIT_SHA.to_string(),
                base_commit_sha: RELEASE_COMMIT_SHA.to_string(),
                merge_base_commit_sha: RELEASE_COMMIT_SHA.to_string(),
                head_commit_sha: BUILD_COMMIT_SHA.to_string(),
                status: CommitComparisonStatus::Ahead,
                ahead_by: 1,
                behind_by: 0,
            },
        );

        let resolved =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("descendant build recurrence");

        assert_eq!(resolved.state, CandidateState::Created);
        assert_eq!(resolved.owner.as_ref().map(|owner| owner.number), Some(46));
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn historical_recurrence_rejects_symbolic_build_refs_before_comparison() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-symbolic-build";
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            TypedRecurrenceEvidence {
                installed_version: None,
                build_commit: Some("develop".to_string()),
                observed_at: "2026-07-15T09:00:00Z".to_string(),
            },
            vec![standard_historical_resolution_comment()],
        );

        let resolved =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("symbolic build proof is a typed ambiguity");

        assert_eq!(resolved.state, CandidateState::Blocked);
        assert_eq!(resolved.blocked_reason, Some(BlockedReason::Ambiguity));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        assert_eq!(
            env.owner_client
                .owner_call_log()
                .iter()
                .filter(|call| call.operation == OwnerRepositoryOperation::CompareCommits)
                .count(),
            1,
            "only the release-tag resolution comparison is allowed"
        );
    }

    #[test]
    fn historical_recurrence_rejects_timestamp_and_ancestry_contradictions() {
        for name in [
            "verified-before-release",
            "comment-before-verification",
            "merge-after-release",
            "release-behind-merge",
            "build-not-strictly-ahead",
        ] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let source = tempfile::tempdir().expect("source");
            let candidate_id = format!("impr-historical-contradiction-{name}");
            let recurrence = if name == "build-not-strictly-ahead" {
                TypedRecurrenceEvidence {
                    installed_version: Some("9.66.0".to_string()),
                    build_commit: Some(BUILD_COMMIT_SHA.to_string()),
                    observed_at: "2026-07-15T09:00:00Z".to_string(),
                }
            } else {
                version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z")
            };
            let comment = match name {
                "verified-before-release" => repository_comment(
                    501,
                    historical_resolution_comment(
                        900,
                        MERGE_COMMIT_SHA,
                        "2026-07-04T00:00:00Z",
                        "v9.65.0",
                    ),
                    "2026-07-10T00:00:00Z",
                ),
                "comment-before-verification" => repository_comment(
                    501,
                    historical_resolution_comment(
                        900,
                        MERGE_COMMIT_SHA,
                        "2026-07-10T00:00:00Z",
                        "v9.65.0",
                    ),
                    "2026-07-09T00:00:00Z",
                ),
                _ => standard_historical_resolution_comment(),
            };
            let (_, mut env) = historical_regression_fixture(
                source.path(),
                &candidate_id,
                recurrence,
                vec![comment],
            );
            let repository = RepositoryIdentity::gwt_upstream();
            match name {
                "merge-after-release" => env.owner_client.seed_merged_pull_request(
                    &repository,
                    MergedPullRequest {
                        number: IssueNumber(900),
                        merge_commit_sha: MERGE_COMMIT_SHA.to_string(),
                        merged_at: "2026-07-06T00:00:00Z".to_string(),
                    },
                ),
                "release-behind-merge" => env.owner_client.seed_commit_comparison(
                    &repository,
                    CommitComparison {
                        base: MERGE_COMMIT_SHA.to_string(),
                        head: "refs/tags/v9.65.0".to_string(),
                        base_commit_sha: MERGE_COMMIT_SHA.to_string(),
                        merge_base_commit_sha: OTHER_COMMIT_SHA.to_string(),
                        head_commit_sha: RELEASE_COMMIT_SHA.to_string(),
                        status: CommitComparisonStatus::Behind,
                        ahead_by: 0,
                        behind_by: 1,
                    },
                ),
                "build-not-strictly-ahead" => env.owner_client.seed_commit_comparison(
                    &repository,
                    CommitComparison {
                        base: RELEASE_COMMIT_SHA.to_string(),
                        head: BUILD_COMMIT_SHA.to_string(),
                        base_commit_sha: RELEASE_COMMIT_SHA.to_string(),
                        merge_base_commit_sha: RELEASE_COMMIT_SHA.to_string(),
                        head_commit_sha: BUILD_COMMIT_SHA.to_string(),
                        status: CommitComparisonStatus::Behind,
                        ahead_by: 0,
                        behind_by: 1,
                    },
                ),
                _ => {}
            }

            let resolved =
                resolve_candidate_owner(&mut env, &candidate_id, CaptureBudgetProfile::Normal)
                    .unwrap_or_else(|error| panic!("{name}: resolution failed: {error}"));

            assert_eq!(resolved.state, CandidateState::Blocked, "{name}");
            assert_eq!(
                resolved.blocked_reason,
                Some(BlockedReason::Ambiguity),
                "{name}"
            );
            assert_eq!(env.owner_client.owner_mutation_count(), 0, "{name}");
        }
    }

    #[test]
    fn historical_identity_transport_failure_is_typed_and_pre_submit() {
        for operation in [
            OwnerRepositoryOperation::FetchMergedPullRequest,
            OwnerRepositoryOperation::CompareCommits,
        ] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let source = tempfile::tempdir().expect("source");
            let candidate_id =
                format!("impr-historical-transport-{operation:?}").to_ascii_lowercase();
            let (_, mut env) = historical_regression_fixture(
                source.path(),
                &candidate_id,
                version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
                vec![standard_historical_resolution_comment()],
            );
            env.owner_client.fail_next_owner_operation(
                operation,
                OwnerRepositoryFaultTiming::BeforeSubmit,
                GitHubApiError::Parse {
                    operation: "historical identity readback".to_string(),
                    message: "identity mismatch".to_string(),
                },
            );

            let resolved =
                resolve_candidate_owner(&mut env, &candidate_id, CaptureBudgetProfile::Normal)
                    .expect("typed transport failure is persisted");

            assert_eq!(resolved.state, CandidateState::Blocked);
            assert_eq!(resolved.blocked_reason, Some(BlockedReason::Parse));
            assert_eq!(env.owner_client.owner_mutation_count(), 0);
        }
    }

    #[test]
    fn regression_create_remote_outcome_is_adopted_without_a_second_create() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-remote-recovery";
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "create regression owner".to_string(),
            },
        );

        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown result is persisted");
        assert_eq!(
            unknown.state,
            CandidateState::RemoteOutcomeUnknown,
            "{unknown:?}; mutations={:?}",
            env.owner_client.owner_mutation_call_log()
        );
        assert!(matches!(
            unknown.attempt.as_ref().map(|attempt| &attempt.intent),
            Some(ResolutionAttemptIntent::CreateRegressionIssue { .. })
        ));
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        let adopted = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("retry adopts the submitted regression owner");

        assert_eq!(adopted.state, CandidateState::Created);
        assert_eq!(adopted.owner.as_ref().map(|owner| owner.number), Some(46));
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn plain_recovery_preserves_unnumbered_root_when_visible_owner_payload_mismatches() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-plain-delayed-created-owner";
        let nonce = "0".repeat(64);
        let (fingerprint, _) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "plain-create-event");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Open,
            "unrelated owner",
        ));
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "create plain owner".to_string(),
            },
        );

        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown create result is persisted");
        let submitted_intent = unknown
            .pending_create_resolution
            .clone()
            .expect("durable submitted create root");
        assert_eq!(
            unknown.state,
            CandidateState::RemoteOutcomeUnknown,
            "{unknown:?}; mutations={:?}",
            env.owner_client.owner_mutation_call_log()
        );

        env.owner_client.seed_repository_issue(owner_issue(
            46,
            IssueState::Open,
            "created owner is hidden from the fingerprint corpus",
        ));
        env.owner_client.seed_repository_issue(owner_issue(
            47,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));

        let retried = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("mismatched visible owner must remain recoverable");

        assert_eq!(
            retried.state,
            CandidateState::RemoteOutcomeUnknown,
            "{retried:?}; mutations={:?}",
            env.owner_client.owner_mutation_call_log()
        );
        assert_eq!(retried.pending_create_resolution, Some(submitted_intent));
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn generic_pre_submit_settlement_preserves_an_existing_create_root() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-create-root-renewal-failure";
        let nonce = "0".repeat(64);
        store_projection_candidate(
            source.path(),
            &nonce,
            candidate_id,
            "create-root-renewal-event",
        );
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            77,
            IssueState::Open,
            "unrelated public issue",
        ));
        let ResolutionAttemptStart::Acquired {
            mut candidate,
            token,
            ..
        } = begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
            .expect("resolution attempt");
        let preflight = owner_resolution_preflight_deferred(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::current(),
            &NoSemanticOwnerAdvisor,
            &resolution_deadline(),
        )
        .expect("plain create authorization");
        let OwnerPreflightOutcome::CreateAuthorized { authorization } = preflight else {
            panic!("expected plain create authorization: {preflight:?}");
        };
        let create_intent = new_owner_attempt_intent(&authorization);
        mark_resolution_attempt_submitted(
            source.path(),
            &mut candidate,
            &token,
            create_intent.clone(),
        )
        .expect("persist submitted create root");

        let settled = settle_owner_commit_error(
            &mut env,
            candidate,
            &token,
            pre_submit_commit_error(
                BlockedReason::Store,
                "RELOAD_CANDIDATE_STORE",
                owner_projection_error("attempt lease expired before renewal"),
            ),
        )
        .expect("generic pre-submit failure is persisted");

        assert_eq!(settled.state, CandidateState::Blocked);
        assert_eq!(
            settled.pending_create_resolution,
            Some(create_intent.clone())
        );

        let retried = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("pending plain create root remains recoverable");

        assert_eq!(retried.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(retried.pending_create_resolution, Some(create_intent));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
    }

    #[test]
    fn generic_pre_submit_settlement_prevents_regression_create_resubmission() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-regression-root-renewal-failure";
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        let ResolutionAttemptStart::Acquired {
            mut candidate,
            token,
            ..
        } = begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
            .expect("resolution attempt");
        let preflight = owner_resolution_preflight_deferred(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::current(),
            &NoSemanticOwnerAdvisor,
            &resolution_deadline(),
        )
        .expect("regression create authorization");
        let OwnerPreflightOutcome::RegressionCreateAuthorized { authorization } = preflight else {
            panic!("expected regression create authorization: {preflight:?}");
        };
        let create_intent = regression_owner_attempt_intent(&authorization);
        mark_resolution_attempt_submitted(
            source.path(),
            &mut candidate,
            &token,
            create_intent.clone(),
        )
        .expect("persist submitted regression create root");

        let settled = settle_owner_commit_error(
            &mut env,
            candidate,
            &token,
            pre_submit_commit_error(
                BlockedReason::Store,
                "RELOAD_CANDIDATE_STORE",
                owner_projection_error("attempt lease expired before renewal"),
            ),
        )
        .expect("generic pre-submit failure is persisted");

        assert_eq!(settled.state, CandidateState::Blocked);
        assert_eq!(
            settled.pending_create_resolution,
            Some(create_intent.clone())
        );

        let retried = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("pending regression create root remains recoverable");

        assert_eq!(retried.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(retried.pending_create_resolution, Some(create_intent));
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
    }

    #[test]
    fn unnumbered_pending_create_root_reconciles_multiple_matching_owners() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-create-root-multiple";
        let nonce = "0".repeat(64);
        store_projection_candidate(
            source.path(),
            &nonce,
            candidate_id,
            "independent-owner-event",
        );
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            77,
            IssueState::Open,
            "unrelated public issue",
        ));
        let ResolutionAttemptStart::Acquired {
            mut candidate,
            token,
            ..
        } = begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
            .expect("resolution attempt");
        let preflight = owner_resolution_preflight_deferred(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::current(),
            &NoSemanticOwnerAdvisor,
            &resolution_deadline(),
        )
        .expect("plain create authorization");
        let OwnerPreflightOutcome::CreateAuthorized { authorization } = preflight else {
            panic!("expected plain create authorization: {preflight:?}");
        };
        let payload = authorization.payload.clone();
        let create_intent = new_owner_attempt_intent(&authorization);
        mark_resolution_attempt_submitted(
            source.path(),
            &mut candidate,
            &token,
            create_intent.clone(),
        )
        .expect("persist unnumbered create root");
        settle_owner_commit_error(
            &mut env,
            candidate,
            &token,
            pre_submit_commit_error(
                BlockedReason::Store,
                "RELOAD_CANDIDATE_STORE",
                owner_projection_error("attempt lease expired before renewal"),
            ),
        )
        .expect("persist blocked unnumbered create root");

        let repository = RepositoryIdentity::gwt_upstream();
        for number in [78, 79] {
            env.owner_client.seed_repository_issue(RepositoryIssue {
                repository: repository.clone(),
                number: IssueNumber(number),
                title: payload.title.clone(),
                body: payload.body.clone(),
                labels: Vec::new(),
                state: IssueState::Open,
                kind: RepositoryIssueKind::Plain,
                updated_at: UpdatedAt::new(format!("u{number}")),
            });
        }
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CloseIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "close duplicate owner".to_string(),
            },
        );

        let partial = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("submitted duplicate close remains recoverable");
        assert_eq!(partial.state, CandidateState::RemoteOutcomeUnknown);

        let resolved =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("multiple matching create results reconcile after partial success");

        assert_eq!(resolved.state, CandidateState::Linked);
        assert_eq!(resolved.owner.as_ref().map(|owner| owner.number), Some(78));
        assert!(resolved.pending_create_resolution.is_none());
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(79), &resolution_deadline())
                .expect("duplicate owner readback")
                .state,
            IssueState::Closed
        );
        assert_eq!(
            env.owner_client
                .owner_mutation_call_log()
                .iter()
                .filter(|call| call.operation == OwnerRepositoryOperation::CreateIssue)
                .count(),
            0
        );
    }

    #[test]
    fn open_readback_after_unknown_duplicate_close_retries_the_idempotent_close() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-create-root-open-close-readback";
        let nonce = "0".repeat(64);
        store_projection_candidate(
            source.path(),
            &nonce,
            candidate_id,
            "open-close-readback-event",
        );
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            77,
            IssueState::Open,
            "unrelated public issue",
        ));
        let ResolutionAttemptStart::Acquired {
            mut candidate,
            token,
            ..
        } = begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
            .expect("resolution attempt");
        let preflight = owner_resolution_preflight_deferred(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::current(),
            &NoSemanticOwnerAdvisor,
            &resolution_deadline(),
        )
        .expect("plain create authorization");
        let OwnerPreflightOutcome::CreateAuthorized { authorization } = preflight else {
            panic!("expected plain create authorization: {preflight:?}");
        };
        let payload = authorization.payload.clone();
        let create_intent = new_owner_attempt_intent(&authorization);
        mark_resolution_attempt_submitted(source.path(), &mut candidate, &token, create_intent)
            .expect("persist unnumbered create root");
        settle_owner_commit_error(
            &mut env,
            candidate,
            &token,
            pre_submit_commit_error(
                BlockedReason::Store,
                "RELOAD_CANDIDATE_STORE",
                owner_projection_error("attempt lease expired before renewal"),
            ),
        )
        .expect("persist blocked unnumbered create root");

        let repository = RepositoryIdentity::gwt_upstream();
        for number in [78, 79] {
            env.owner_client.seed_repository_issue(RepositoryIssue {
                repository: repository.clone(),
                number: IssueNumber(number),
                title: payload.title.clone(),
                body: payload.body.clone(),
                labels: Vec::new(),
                state: IssueState::Open,
                kind: RepositoryIssueKind::Plain,
                updated_at: UpdatedAt::new(format!("u{number}")),
            });
        }
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CloseIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "close duplicate owner".to_string(),
            },
        );

        let partial = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("submitted duplicate close remains recoverable");
        assert_eq!(partial.state, CandidateState::RemoteOutcomeUnknown);
        env.owner_client.seed_repository_issue(RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(79),
            title: payload.title,
            body: payload.body,
            labels: Vec::new(),
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new("authoritative-open-readback"),
        });

        let resolved =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("authoritative open duplicate is closed idempotently on retry");

        assert_eq!(resolved.state, CandidateState::Linked, "{resolved:?}");
        assert_eq!(resolved.owner.as_ref().map(|owner| owner.number), Some(78));
        assert_eq!(
            env.owner_client
                .owner_mutation_call_log()
                .iter()
                .filter(|call| call.operation == OwnerRepositoryOperation::CloseIssue)
                .count(),
            2
        );
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(79), &resolution_deadline())
                .expect("duplicate owner readback")
                .state,
            IssueState::Closed
        );
    }

    #[test]
    fn contract_mapped_occurrence_response_loss_adopts_comment_without_body_fingerprint() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-contract-comment-response-loss";
        let nonce = "0".repeat(64);
        let (fingerprint, _) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "route-event-a");
        crate::cli::improvement_store::update(source.path(), |store| {
            let occurrence = &mut store.candidates[0].distinct_occurrences[0];
            occurrence.producer_id = Some("test.owner-route.v1".to_string());
            occurrence.routing_basis_revision = Some(9);
            occurrence.opaque_key =
                opaque_occurrence_key(&nonce, &fingerprint, "test.owner-route.v1", "route-event-a");
            Ok(())
        })
        .expect("route candidate through the pinned contract owner");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            77,
            IssueState::Open,
            "revision-pinned contract owner without a fingerprint body marker",
        ));
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateComment,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "contract owner occurrence response".to_string(),
            },
        );

        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("submitted contract owner comment remains recoverable");
        assert_eq!(
            unknown.state,
            CandidateState::RemoteOutcomeUnknown,
            "{unknown:?}; mutations={:?}",
            env.owner_client.owner_mutation_call_log()
        );
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        let recovered =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("authoritative contract owner comment is adopted");

        assert_eq!(recovered.state, CandidateState::Linked, "{recovered:?}");
        assert_eq!(recovered.owner.as_ref().map(|owner| owner.number), Some(77));
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn closed_duplicate_without_reconciliation_marker_cannot_clear_latch() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-reconciliation-marker-cleanup";
        let nonce = "0".repeat(64);
        let (fingerprint, _) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "cleanup-marker");
        crate::cli::improvement_store::update(source.path(), |store| {
            let candidate = &mut store.candidates[0];
            candidate.reconciliation_required = true;
            candidate.reconciliation_owner_numbers = vec![78, 79];
            Ok(())
        })
        .expect("arm durable reconciliation latch");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            78,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        env.owner_client.seed_repository_issue(owner_issue(
            79,
            IssueState::Closed,
            fingerprint_marker(&fingerprint),
        ));

        let blocked = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("missing reconciliation proof fails closed");

        assert_eq!(blocked.state, CandidateState::Blocked, "{blocked:?}");
        assert_eq!(blocked.blocked_reason, Some(BlockedReason::Reconciliation));
        assert!(blocked.reconciliation_required);
        assert_eq!(env.owner_client.owner_mutation_count(), 0);

        let payload = render_reconciliation_comment_payload(
            &blocked,
            78,
            79,
            &PublicMutationContext::for_repo(source.path()),
        )
        .expect("reconciliation payload");
        env.owner_client.seed_repository_comments(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(79),
            vec![repository_comment(
                700,
                payload.body,
                "2026-07-16T00:00:00Z",
            )],
        );

        let linked = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("closed duplicate cleanup with marker proof");

        assert_eq!(linked.state, CandidateState::Linked, "{linked:?}");
        assert!(!linked.reconciliation_required);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn recovered_reconciliation_comment_survives_a_non_monotonic_preflight_view() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-reconciliation-non-monotonic-comments";
        let nonce = "0".repeat(64);
        let (fingerprint, _) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "event-nm-a");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        for number in [78, 79] {
            env.owner_client.seed_repository_issue(owner_issue(
                number,
                IssueState::Open,
                fingerprint_marker(&fingerprint),
            ));
        }
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateComment,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "reconciliation comment response".to_string(),
            },
        );

        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown reconciliation response is persisted");
        assert_eq!(
            unknown.state,
            CandidateState::RemoteOutcomeUnknown,
            "{unknown:?}; mutations={:?}",
            env.owner_client.owner_mutation_call_log()
        );
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
        let repository = RepositoryIdentity::gwt_upstream();
        let submitted = env
            .owner_client
            .list_comments(&repository, IssueNumber(79), &resolution_deadline())
            .expect("submitted reconciliation comment")
            .items()
            .to_vec();
        env.owner_client.queue_owner_comment_views(
            &repository,
            IssueNumber(79),
            [submitted, Vec::new()],
        );

        let linked = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("settled marker evidence is retained for the retry");

        assert_eq!(linked.state, CandidateState::Linked, "{linked:?}");
        assert_eq!(env.owner_client.owner_mutation_count(), 3);
        let comments = env
            .owner_client
            .list_comments(&repository, IssueNumber(79), &resolution_deadline())
            .expect("duplicate comments");
        assert_eq!(
            comments
                .items()
                .iter()
                .filter(|comment| comment.body.contains("gwt:improvement-reconciliation:v1"))
                .count(),
            1
        );
    }

    #[test]
    fn plain_create_recovery_uses_submitted_remote_bytes_after_candidate_evidence_changes() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-create-journal-evidence-change";
        let nonce = "0".repeat(64);
        store_projection_candidate(source.path(), &nonce, candidate_id, "create-journal-a");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            77,
            IssueState::Open,
            "unrelated public issue",
        ));
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "plain create response".to_string(),
            },
        );

        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown plain create is persisted");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        replace_candidate_public_outcomes(
            source.path(),
            "BOARD_STATUS_RESTORED",
            "BOARD_STATUS_DELAYED",
        );

        let adopted = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("submitted plain issue bytes remain authoritative");

        assert_eq!(adopted.state, CandidateState::Created, "{adopted:?}");
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn occurrence_recovery_uses_submitted_remote_bytes_after_candidate_evidence_changes() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-occurrence-journal-evidence-change";
        let nonce = "0".repeat(64);
        let (fingerprint, _) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "comment-journal-a");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateComment,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "occurrence comment response".to_string(),
            },
        );

        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown occurrence comment is persisted");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        replace_candidate_public_outcomes(
            source.path(),
            "BOARD_STATUS_RESTORED",
            "BOARD_STATUS_DELAYED",
        );

        let linked = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("submitted occurrence bytes remain authoritative");

        assert_eq!(linked.state, CandidateState::Linked, "{linked:?}");
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn reconciliation_recovery_uses_marker_identity_after_candidate_evidence_changes() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-reconciliation-journal-evidence-change";
        let nonce = "0".repeat(64);
        let (fingerprint, _) = store_projection_candidate(
            source.path(),
            &nonce,
            candidate_id,
            "reconciliation-journal-a",
        );
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        for number in [78, 79] {
            env.owner_client.seed_repository_issue(owner_issue(
                number,
                IssueState::Open,
                fingerprint_marker(&fingerprint),
            ));
        }
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateComment,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "reconciliation comment response".to_string(),
            },
        );

        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown reconciliation comment is persisted");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        replace_candidate_public_outcomes(
            source.path(),
            "BOARD_STATUS_RESTORED",
            "BOARD_STATUS_DELAYED",
        );

        let linked = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("submitted reconciliation marker remains authoritative");

        assert_eq!(linked.state, CandidateState::Linked, "{linked:?}");
        assert_eq!(env.owner_client.owner_mutation_count(), 3);
        let comments = env
            .owner_client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(79),
                &resolution_deadline(),
            )
            .expect("duplicate comments");
        assert_eq!(
            comments
                .items()
                .iter()
                .filter(|comment| comment.body.contains("gwt:improvement-reconciliation:v1"))
                .count(),
            1
        );
    }

    #[test]
    fn settled_occurrence_recovery_is_not_rearmed_when_the_owner_closes() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-settled-comment-owner-closed";
        let nonce = "0".repeat(64);
        let (fingerprint, _) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "settled-event-a");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateComment,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "occurrence comment response".to_string(),
            },
        );

        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown occurrence comment is persisted");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Closed,
            fingerprint_marker(&fingerprint),
        ));

        let blocked = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("settled comment is not restored after historical preflight");

        assert_eq!(blocked.state, CandidateState::Blocked, "{blocked:?}");
        assert_eq!(blocked.blocked_reason, Some(BlockedReason::Ambiguity));
        assert!(blocked.attempt.is_none());
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn regression_create_recovery_projects_a_recurrence_added_while_remote_unknown() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-regression-recovery-new-recurrence";
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "regression create response".to_string(),
            },
        );

        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown regression create is persisted");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        let added_occurrence =
            append_projection_occurrence(source.path(), &"0".repeat(64), "recurrence-event-b");

        let recovered =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("created regression owner absorbs the later recurrence");

        assert_eq!(recovered.state, CandidateState::Created, "{recovered:?}");
        assert_eq!(recovered.owner.as_ref().map(|owner| owner.number), Some(46));
        assert_eq!(recovered.occurrences, 2);
        assert_eq!(env.owner_client.owner_mutation_count(), 2);
        let comments = env
            .owner_client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(46),
                &resolution_deadline(),
            )
            .expect("regression owner occurrence comments");
        assert!(comments
            .items()
            .iter()
            .any(|comment| comment_matches_occurrence(
                comment,
                &added_occurrence,
                recovered.fingerprint.as_deref().expect("fingerprint")
            )));
    }

    #[test]
    fn regression_create_recovery_adopts_a_lost_additional_occurrence_comment_response() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-regression-recovery-comment-response-loss";
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "regression create response".to_string(),
            },
        );
        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown regression create is persisted");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        let added_occurrence = append_projection_occurrence(
            source.path(),
            &"0".repeat(64),
            "lost-comment-recurrence-event",
        );
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateComment,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "additional occurrence comment response".to_string(),
            },
        );

        let comment_unknown =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("unknown additional comment response is persisted");
        assert_eq!(comment_unknown.state, CandidateState::RemoteOutcomeUnknown);

        let recovered =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("submitted additional comment is adopted without a second mutation");

        assert_eq!(recovered.state, CandidateState::Created, "{recovered:?}");
        assert_eq!(recovered.occurrences, 2);
        assert_eq!(env.owner_client.owner_mutation_count(), 2);
        let comments = env
            .owner_client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(46),
                &resolution_deadline(),
            )
            .expect("regression owner occurrence comments");
        assert_eq!(
            comments
                .items()
                .iter()
                .filter(|comment| comment_matches_occurrence(
                    comment,
                    &added_occurrence,
                    recovered.fingerprint.as_deref().expect("fingerprint")
                ))
                .count(),
            1
        );
    }

    #[test]
    fn regression_create_recovery_resumes_after_additional_comment_readback_failures() {
        for (name, list_comments_call) in [("before-comment", 3), ("final-readback", 5)] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let source = tempfile::tempdir().expect("source");
            let candidate_id = format!("impr-regression-recovery-{name}");
            let (_, mut env) = historical_regression_fixture(
                source.path(),
                &candidate_id,
                version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
                vec![standard_historical_resolution_comment()],
            );
            env.owner_client.fail_next_owner_operation(
                OwnerRepositoryOperation::CreateIssue,
                OwnerRepositoryFaultTiming::AfterSubmit,
                GitHubApiError::Timeout {
                    operation: "regression create response".to_string(),
                },
            );
            let unknown =
                resolve_candidate_owner(&mut env, &candidate_id, CaptureBudgetProfile::Normal)
                    .expect("unknown regression create is persisted");
            assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
            append_projection_occurrence(
                source.path(),
                &"0".repeat(64),
                &format!("{name}-occurrence-event"),
            );
            env.owner_client.fail_owner_operation_on_nth(
                OwnerRepositoryOperation::ListComments,
                list_comments_call,
                OwnerRepositoryFaultTiming::BeforeSubmit,
                GitHubApiError::Timeout {
                    operation: format!("{name} occurrence comment readback"),
                },
            );

            let readback_unknown =
                resolve_candidate_owner(&mut env, &candidate_id, CaptureBudgetProfile::Normal)
                    .expect("additional comment readback failure is persisted");
            assert_eq!(
                readback_unknown.state,
                CandidateState::RemoteOutcomeUnknown,
                "{name}: {readback_unknown:?}"
            );

            let recovered =
                resolve_candidate_owner(&mut env, &candidate_id, CaptureBudgetProfile::Normal)
                    .expect("retry resumes from the durable regression create root");

            assert_eq!(
                recovered.state,
                CandidateState::Created,
                "{name}: {recovered:?}"
            );
            assert_eq!(recovered.occurrences, 2);
            assert_eq!(env.owner_client.owner_mutation_count(), 2);
        }
    }

    #[test]
    fn regression_create_recovery_projects_an_added_non_recurrence_occurrence() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-regression-recovery-new-standard-occurrence";
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "regression create response".to_string(),
            },
        );
        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown regression create is persisted");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        let added_occurrence = append_projection_occurrence(
            source.path(),
            &"0".repeat(64),
            "standard-occurrence-event",
        );
        crate::cli::improvement_store::update(source.path(), |store| {
            let candidate = store.candidates.first_mut().expect("source candidate");
            let evidence = candidate.typed_evidence.as_ref().expect("typed evidence");
            let occurrence = candidate
                .distinct_occurrences
                .iter_mut()
                .find(|occurrence| occurrence.opaque_key == added_occurrence)
                .expect("added occurrence");
            occurrence.recurrence = None;
            occurrence.evidence_digest = occurrence_evidence_digest(evidence, None);
            Ok(())
        })
        .expect("remove recurrence metadata from the added occurrence");

        let recovered =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("created regression owner absorbs the standard occurrence");

        assert_eq!(recovered.state, CandidateState::Created, "{recovered:?}");
        assert_eq!(recovered.occurrences, 2);
        assert_eq!(env.owner_client.owner_mutation_count(), 2);
        let comments = env
            .owner_client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(46),
                &resolution_deadline(),
            )
            .expect("regression owner occurrence comments");
        assert!(comments.items().iter().any(|comment| {
            comment_matches_occurrence(
                comment,
                &added_occurrence,
                recovered.fingerprint.as_deref().expect("fingerprint"),
            )
        }));
    }

    #[test]
    fn active_owner_partial_comment_success_retries_only_the_unsent_occurrence() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-active-owner-partial-comments";
        let nonce = "0".repeat(64);
        let (fingerprint, _) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "partial-comment-a");
        crate::cli::improvement_store::update(source.path(), |store| {
            let candidate = &mut store.candidates[0];
            let mut second = candidate.distinct_occurrences[0].clone();
            second.opaque_key = opaque_occurrence_key(
                &"0".repeat(64),
                &fingerprint,
                "test.coordination-gate.v1",
                "partial-comment-b",
            );
            second.captured_at = "2026-07-15T00:00:00Z".to_string();
            second.replay_proof = Some(OccurrenceReplayProof::RegisteredEvent {
                source_scope_nonce: "0".repeat(64),
                source_event_id: "partial-comment-b".to_string(),
            });
            candidate.distinct_occurrences.push(second);
            candidate.occurrences = 2;
            Ok(())
        })
        .expect("append second occurrence");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        env.owner_client.fail_owner_operation_on_nth(
            OwnerRepositoryOperation::CreateComment,
            2,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            GitHubApiError::Network("connection refused before submit".to_string()),
        );

        let partial = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("partial comment result is persisted");
        assert_eq!(partial.state, CandidateState::Blocked);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        let recovered =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("retry creates only the unsent comment");

        assert_eq!(recovered.state, CandidateState::Linked);
        assert_eq!(env.owner_client.owner_mutation_count(), 2);
        let comments = env
            .owner_client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(45),
                &resolution_deadline(),
            )
            .expect("authoritative comments");
        assert_eq!(comments.items().len(), 2);
    }

    #[test]
    fn regression_recovery_preserves_unnumbered_submitted_intent_when_visible_owner_mismatches() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-delayed-regression-owner";
        let (fingerprint, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "create regression owner".to_string(),
            },
        );
        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown result is persisted");
        let submitted_intent = unknown
            .attempt
            .as_ref()
            .expect("submitted attempt")
            .intent
            .clone();

        env.owner_client.seed_repository_issue(owner_issue(
            46,
            IssueState::Open,
            "created owner is not visible in the authoritative fingerprint corpus",
        ));
        env.owner_client.seed_repository_issue(owner_issue(
            47,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));

        let retried = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("mismatched visibility must remain recoverable");

        assert_eq!(retried.state, CandidateState::RemoteOutcomeUnknown);
        assert!(retried.pending_create_resolution.is_some());
        let attempt = retried
            .attempt
            .as_ref()
            .expect("preserved recovery attempt");
        assert_eq!(attempt.remote_phase, AttemptRemotePhase::Submitted);
        assert_eq!(attempt.intent, submitted_intent);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn regression_recovery_fetch_failure_preserves_complete_submitted_intent() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-recovery-fetch-failure";
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "create regression owner".to_string(),
            },
        );
        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown result is persisted");
        let submitted_intent = unknown
            .attempt
            .as_ref()
            .expect("submitted attempt")
            .intent
            .clone();
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::FetchIssue,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            GitHubApiError::Timeout {
                operation: "fetch submitted regression owner".to_string(),
            },
        );

        let retried = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("fetch failure must remain recoverable");

        assert_eq!(retried.state, CandidateState::RemoteOutcomeUnknown);
        assert!(retried.pending_create_resolution.is_some());
        let attempt = retried
            .attempt
            .as_ref()
            .expect("preserved recovery attempt");
        assert_eq!(attempt.remote_phase, AttemptRemotePhase::Submitted);
        assert_eq!(attempt.intent, submitted_intent);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        let adopted = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("retry after fetch recovery");

        assert_eq!(adopted.state, CandidateState::Created);
        assert!(adopted.pending_create_resolution.is_none());
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn regression_recovery_projection_failure_preserves_complete_submitted_intent() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-recovery-projection-failure";
        let (_, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "create regression owner".to_string(),
            },
        );
        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown result is persisted");
        let submitted_intent = unknown
            .attempt
            .as_ref()
            .expect("submitted attempt")
            .intent
            .clone();
        crate::cli::improvement_store::fail_next_owner_projection_commit()
            .expect("arm projection failure");

        let retried = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("projection failure must remain recoverable");

        assert_eq!(retried.state, CandidateState::RemoteOutcomeUnknown);
        assert!(retried.pending_create_resolution.is_some());
        let attempt = retried
            .attempt
            .as_ref()
            .expect("preserved recovery attempt");
        assert_eq!(attempt.remote_phase, AttemptRemotePhase::Submitted);
        assert_eq!(attempt.intent, submitted_intent);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        let adopted = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("retry after projection recovery");

        assert_eq!(adopted.state, CandidateState::Created);
        assert!(adopted.pending_create_resolution.is_none());
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn regression_recovery_reconciles_a_lower_different_payload_owner_and_clears_root() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-lower-regression-owner";
        let (fingerprint, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            GitHubApiError::Timeout {
                operation: "create regression owner".to_string(),
            },
        );
        let unknown = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("unknown result is persisted");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert!(unknown.pending_create_resolution.is_some());
        env.owner_client.seed_repository_issue(owner_issue(
            40,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));

        let resolved =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("lower exact owner reconciliation");

        assert_eq!(resolved.state, CandidateState::Linked, "{resolved:?}");
        assert_eq!(resolved.owner.as_ref().map(|owner| owner.number), Some(40));
        assert!(resolved.pending_create_resolution.is_none());
        assert!(!resolved.reconciliation_required);
        assert!(resolved.reconciliation_owner_numbers.is_empty());
        assert_eq!(env.owner_client.owner_mutation_count(), 4);
        let repository = RepositoryIdentity::gwt_upstream();
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(46), &resolution_deadline())
                .expect("created duplicate readback")
                .state,
            IssueState::Closed
        );
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(45), &resolution_deadline())
                .expect("historical owner readback")
                .state,
            IssueState::Closed
        );
        let projection = crate::cli::improvement_contract::read_owner_projection()
            .expect("canonical regression projection");
        assert_eq!(projection.owners.len(), 1);
        assert_eq!(projection.owners[0].owner.number, 40);
        assert_eq!(projection.owners[0].aggregate_count, 1);
        assert!(env
            .owner_client
            .list_comments(&repository, IssueNumber(40), &resolution_deadline())
            .expect("canonical owner comments")
            .items()
            .iter()
            .any(|comment| comment.body.contains("gwt:improvement-occurrence:v1")));
    }

    #[test]
    fn regression_postflight_uses_recorded_create_root_to_adopt_lower_canonical_owner() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-postflight-lower-owner";
        let (fingerprint, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        let repository = RepositoryIdentity::gwt_upstream();
        let historical_owner =
            owner_issue(45, IssueState::Closed, fingerprint_marker(&fingerprint));
        env.owner_client.seed_repository_issue(owner_issue(
            40,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![historical_owner.clone()], "initial-before-create"),
                (vec![historical_owner.clone()], "initial-before-create"),
                (vec![historical_owner.clone()], "refresh-before-create"),
                (vec![historical_owner], "refresh-before-create"),
            ],
        );

        let resolved =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("postflight duplicate reconciliation");

        assert_eq!(resolved.state, CandidateState::Linked, "{resolved:?}");
        assert_eq!(resolved.owner.as_ref().map(|owner| owner.number), Some(40));
        assert!(resolved.pending_create_resolution.is_none());
        assert!(!resolved.reconciliation_required);
        assert!(resolved.reconciliation_owner_numbers.is_empty());
        assert_eq!(env.owner_client.owner_mutation_count(), 4);
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(46), &resolution_deadline())
                .expect("created duplicate readback")
                .state,
            IssueState::Closed
        );
        assert!(env
            .owner_client
            .list_comments(&repository, IssueNumber(40), &resolution_deadline())
            .expect("canonical owner comments")
            .items()
            .iter()
            .any(|comment| comment.body.contains("gwt:improvement-occurrence:v1")));
    }

    #[test]
    fn regression_projection_uses_authorized_order_for_multiple_recurrences() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-multiple-recurrences";
        let (fingerprint, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );
        crate::cli::improvement_store::update(source.path(), |store| {
            let candidate = &mut store.candidates[0];
            let mut second = candidate.distinct_occurrences[0].clone();
            let second_event = "historical-regression-second-event";
            second.opaque_key = opaque_occurrence_key(
                &"0".repeat(64),
                &fingerprint,
                "test.coordination-gate.v1",
                second_event,
            );
            second.captured_at = "2026-07-15T10:00:00Z".to_string();
            second.replay_proof = Some(OccurrenceReplayProof::RegisteredEvent {
                source_scope_nonce: "0".repeat(64),
                source_event_id: second_event.to_string(),
            });
            second.recurrence = Some(version_recurrence(Some("9.66.1"), "2026-07-15T10:00:00Z"));
            second.evidence_digest = occurrence_evidence_digest(
                candidate.typed_evidence.as_ref().expect("typed evidence"),
                second.recurrence.as_ref(),
            );
            candidate.distinct_occurrences.push(second);
            candidate
                .distinct_occurrences
                .sort_by(|left, right| right.opaque_key.cmp(&left.opaque_key));
            candidate.occurrences = candidate.distinct_occurrences.len() as u64;
            Ok(())
        })
        .expect("append second recurrence");

        let resolved =
            resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
                .expect("multiple recurrence resolution");

        assert_eq!(resolved.state, CandidateState::Created);
        assert_eq!(resolved.owner.as_ref().map(|owner| owner.number), Some(46));
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn regression_generation_uses_current_durable_owner_and_only_newer_occurrences() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-historical-second-regression-generation";
        let (fingerprint, mut env) = historical_regression_fixture(
            source.path(),
            candidate_id,
            version_recurrence(Some("9.66.0"), "2026-07-15T09:00:00Z"),
            vec![standard_historical_resolution_comment()],
        );

        let first = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("first regression owner");
        assert_eq!(first.state, CandidateState::Created);
        assert_eq!(first.owner.as_ref().map(|owner| owner.number), Some(46));

        let repository = RepositoryIdentity::gwt_upstream();
        let mut first_regression = env
            .owner_client
            .fetch_issue(&repository, IssueNumber(46), &resolution_deadline())
            .expect("first regression readback");
        first_regression.state = IssueState::Closed;
        env.owner_client.seed_repository_issue(first_regression);
        env.owner_client.seed_repository_comments(
            &repository,
            IssueNumber(46),
            vec![repository_comment(
                502,
                historical_resolution_comment(
                    901,
                    OTHER_COMMIT_SHA,
                    "2026-07-20T00:00:00Z",
                    "v9.66.0",
                ),
                "2026-07-20T00:00:00Z",
            )],
        );
        env.owner_client.seed_merged_pull_request(
            &repository,
            MergedPullRequest {
                number: IssueNumber(901),
                merge_commit_sha: OTHER_COMMIT_SHA.to_string(),
                merged_at: "2026-07-16T00:00:00Z".to_string(),
            },
        );
        env.owner_client.seed_repository_release(
            &repository,
            RepositoryRelease {
                tag_name: "v9.66.0".to_string(),
                target_commitish: "develop".to_string(),
                published_at: "2026-07-18T00:00:00Z".to_string(),
            },
        );
        env.owner_client.seed_commit_comparison(
            &repository,
            CommitComparison {
                base: OTHER_COMMIT_SHA.to_string(),
                head: "refs/tags/v9.66.0".to_string(),
                base_commit_sha: OTHER_COMMIT_SHA.to_string(),
                merge_base_commit_sha: OTHER_COMMIT_SHA.to_string(),
                head_commit_sha: CONFLICTING_COMMIT_SHA.to_string(),
                status: CommitComparisonStatus::Ahead,
                ahead_by: 2,
                behind_by: 0,
            },
        );
        crate::cli::improvement_store::update(source.path(), |store| {
            let candidate = &mut store.candidates[0];
            let mut second = candidate.distinct_occurrences[0].clone();
            let second_event = "historical-regression-next-generation";
            second.opaque_key = opaque_occurrence_key(
                &"0".repeat(64),
                &fingerprint,
                "test.coordination-gate.v1",
                second_event,
            );
            second.captured_at = "2026-07-21T09:00:00Z".to_string();
            second.replay_proof = Some(OccurrenceReplayProof::RegisteredEvent {
                source_scope_nonce: "0".repeat(64),
                source_event_id: second_event.to_string(),
            });
            second.recurrence = Some(version_recurrence(Some("9.67.0"), "2026-07-21T09:00:00Z"));
            second.evidence_digest = occurrence_evidence_digest(
                candidate.typed_evidence.as_ref().expect("typed evidence"),
                second.recurrence.as_ref(),
            );
            candidate.distinct_occurrences.push(second);
            candidate
                .distinct_occurrences
                .sort_by(|left, right| right.opaque_key.cmp(&left.opaque_key));
            candidate.occurrences = candidate.distinct_occurrences.len() as u64;
            candidate.state = CandidateState::Recurrent;
            Ok(())
        })
        .expect("append next-generation recurrence");

        let second = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("second regression owner");

        assert_eq!(second.state, CandidateState::Created, "{second:?}");
        assert_eq!(second.owner.as_ref().map(|owner| owner.number), Some(47));
        assert_eq!(env.owner_client.owner_mutation_count(), 2);
        let second_readback = env
            .owner_client
            .fetch_issue(&repository, IssueNumber(47), &resolution_deadline())
            .expect("second regression readback");
        assert!(second_readback
            .body
            .contains("<!-- gwt:improvement-regression:v1 historical:46 proof:"));
    }

    fn blocked_ambiguous_owner_fixture(source: &Path, candidate_id: &str) -> (TestEnv, String) {
        let nonce = "0".repeat(64);
        let (fingerprint, _) =
            store_projection_candidate(source, &nonce, candidate_id, "manual-selection-event");
        let mut env = TestEnv::new(source.join("cache"));
        env.repo_path = source.to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        let mut spec = owner_issue(46, IssueState::Open, fingerprint_marker(&fingerprint));
        spec.kind = RepositoryIssueKind::Spec;
        env.owner_client.seed_repository_issue(spec);
        let blocked = resolve_candidate_owner(&mut env, candidate_id, CaptureBudgetProfile::Normal)
            .expect("ambiguous owner resolution");
        assert_eq!(blocked.state, CandidateState::Blocked);
        assert_eq!(blocked.blocked_reason, Some(BlockedReason::Ambiguity));
        let revision = blocked
            .resolver_snapshot
            .as_ref()
            .expect("resolver snapshot")
            .resolver_revision
            .clone();
        (env, revision)
    }

    #[test]
    fn manual_owner_selection_requires_stored_and_fresh_revision_then_audits_safe_link() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-manual-owner-selection";
        let (mut env, revision) = blocked_ambiguous_owner_fixture(source.path(), candidate_id);

        let selected = select_candidate_owner(
            &mut env,
            candidate_id,
            45,
            &revision,
            CaptureBudgetProfile::Normal,
        )
        .expect("fresh selected owner");

        assert_eq!(selected.state, CandidateState::Linked);
        assert_eq!(selected.owner.as_ref().map(|owner| owner.number), Some(45));
        assert!(selected.resolver_snapshot.is_none());
        assert!(matches!(
            selected.audit.last(),
            Some(ImprovementAuditEntry::ManualOwnerSelection {
                owner_number: 45,
                resolver_revision,
                ..
            }) if resolver_revision == &revision
        ));
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn manual_owner_selection_rejects_stale_revision_or_closed_owner_without_mutation() {
        {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let source = tempfile::tempdir().expect("source");
            let candidate_id = "impr-manual-owner-stored-stale";
            let (mut env, revision) = blocked_ambiguous_owner_fixture(source.path(), candidate_id);
            let mut stale_revision = revision.into_bytes();
            stale_revision[0] = if stale_revision[0] == b'a' {
                b'b'
            } else {
                b'a'
            };
            let stale_revision = String::from_utf8(stale_revision).expect("ASCII revision");

            let error = select_candidate_owner(
                &mut env,
                candidate_id,
                45,
                &stale_revision,
                CaptureBudgetProfile::Normal,
            )
            .expect_err("stored revision mismatch must fail");

            assert!(error.to_string().contains("revision"));
            assert_eq!(env.owner_client.owner_mutation_count(), 0);
            let stored = crate::cli::improvement_store::load_and_repair(source.path())
                .expect("candidate store")
                .candidates
                .remove(0);
            assert_eq!(stored.state, CandidateState::Blocked);
            assert!(stored.audit.is_empty());
        }

        for (name, close_selected) in [("stale", false), ("closed", true)] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let source = tempfile::tempdir().expect("source");
            let candidate_id = format!("impr-manual-owner-{name}");
            let (mut env, revision) = blocked_ambiguous_owner_fixture(source.path(), &candidate_id);
            if close_selected {
                let fingerprint = crate::cli::improvement_store::load_and_repair(source.path())
                    .expect("candidate store")
                    .candidates[0]
                    .fingerprint
                    .clone()
                    .expect("fingerprint");
                env.owner_client.seed_repository_issue(owner_issue(
                    45,
                    IssueState::Closed,
                    fingerprint_marker(&fingerprint),
                ));
            } else {
                env.owner_client.seed_repository_issue(owner_issue(
                    99,
                    IssueState::Open,
                    "unrelated generation change",
                ));
            }

            let error = select_candidate_owner(
                &mut env,
                &candidate_id,
                45,
                &revision,
                CaptureBudgetProfile::Normal,
            )
            .expect_err("stale or closed selection must fail");

            assert!(
                error.to_string().contains("revision")
                    || error.to_string().contains("active owner"),
                "{name}: {error}"
            );
            assert_eq!(env.owner_client.owner_mutation_count(), 0, "{name}");
            let stored = crate::cli::improvement_store::load_and_repair(source.path())
                .expect("candidate store")
                .candidates
                .remove(0);
            assert_eq!(stored.state, CandidateState::Blocked, "{name}");
            assert!(stored.audit.is_empty(), "{name}");
        }
    }

    #[test]
    fn manual_owner_selection_does_not_audit_before_authoritative_readback() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-manual-owner-readback-failure";
        let (mut env, revision) = blocked_ambiguous_owner_fixture(source.path(), candidate_id);
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::FetchIssue,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            GitHubApiError::Timeout {
                operation: "manual selection owner readback".to_string(),
            },
        );

        let failed = select_candidate_owner(
            &mut env,
            candidate_id,
            45,
            &revision,
            CaptureBudgetProfile::Normal,
        )
        .expect("failed authoritative readback is persisted as retryable state");

        assert_eq!(failed.state, CandidateState::Blocked);
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        let stored = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("candidate store")
            .candidates
            .remove(0);
        assert!(stored.audit.is_empty());
    }

    #[test]
    fn manual_owner_selection_does_not_audit_before_final_authoritative_readback() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let candidate_id = "impr-manual-owner-final-readback-failure";
        let (mut env, revision) = blocked_ambiguous_owner_fixture(source.path(), candidate_id);
        env.owner_client.fail_owner_operation_on_nth(
            OwnerRepositoryOperation::FetchIssue,
            2,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            GitHubApiError::Timeout {
                operation: "manual selection final owner readback".to_string(),
            },
        );

        let failed = select_candidate_owner(
            &mut env,
            candidate_id,
            45,
            &revision,
            CaptureBudgetProfile::Normal,
        )
        .expect("failed final readback is persisted as retryable state");

        assert_eq!(failed.state, CandidateState::Blocked);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
        let stored = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("candidate store")
            .candidates
            .remove(0);
        assert!(stored.attempt.is_none());
        assert!(stored.audit.is_empty());
    }

    #[test]
    fn owner_commit_and_error_settlement_cannot_overwrite_concurrent_dismissal() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "0".repeat(64);
        let candidate_id = "impr-owner-dismiss-race";
        let (fingerprint, _) =
            store_projection_candidate(source.path(), &nonce, candidate_id, "dismiss-race-event");
        let mut env = TestEnv::new(source.path().join("cache"));
        env.repo_path = source.path().to_path_buf();
        env.improvement_source_scope_nonce = nonce;
        env.owner_client.seed_repository_issue(owner_issue(
            45,
            IssueState::Open,
            fingerprint_marker(&fingerprint),
        ));
        let ResolutionAttemptStart::Acquired {
            mut candidate,
            token,
            ..
        } = begin_resolution_attempt(source.path(), candidate_id, CaptureBudgetProfile::Normal)
            .expect("owner resolution lease");
        let OwnerPreflightOutcome::Active {
            owner, comments, ..
        } = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &ContractRoutingRegistry::default(),
            &NoSemanticOwnerAdvisor,
            &resolution_deadline(),
        )
        .expect("active owner preflight")
        else {
            panic!("expected active owner");
        };
        crate::cli::improvement_store::update(source.path(), |store| {
            let stored = &mut store.candidates[0];
            stored.dismissed_reason = Some("dismissed during owner resolution".to_string());
            transition_candidate(stored, CandidateState::Dismissed)
        })
        .expect("concurrent dismissal");

        let error = commit_active_owner_resolution(
            &mut env,
            &mut candidate,
            &owner,
            &comments,
            &resolution_deadline(),
        )
        .expect_err("stale resolver must reject the dismissed candidate");
        settle_owner_commit_error(&mut env, candidate, &token, error)
            .expect_err("failure persistence must not overwrite dismissal");

        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        let stored = crate::cli::improvement_store::load_and_repair(source.path())
            .expect("candidate store")
            .candidates
            .remove(0);
        assert_eq!(stored.state, CandidateState::Dismissed);
        assert_eq!(
            stored.dismissed_reason.as_deref(),
            Some("dismissed during owner resolution")
        );
        assert!(stored.audit.is_empty());
    }

    #[test]
    fn projection_first_commit_aggregates_two_sources_and_dedupes_source_replay() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source_a = tempfile::tempdir().expect("source A");
        let source_b = tempfile::tempdir().expect("source B");
        let (fingerprint, occurrence_a) = store_projection_candidate(
            source_a.path(),
            &"1".repeat(64),
            "impr-source-a",
            "same-producer-event",
        );
        let (_, occurrence_b) = store_projection_candidate(
            source_b.path(),
            &"2".repeat(64),
            "impr-source-b",
            "same-producer-event",
        );
        assert_ne!(occurrence_a, occurrence_b);
        let owner = verified_projection_owner(3164, &fingerprint);
        let binding_a = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-source-a".to_string(),
            owner: owner.clone(),
            occurrence_key: occurrence_a,
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T08:01:00Z".to_string(),
        };
        let binding_b = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-source-b".to_string(),
            owner,
            occurrence_key: occurrence_b,
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T08:02:00Z".to_string(),
        };

        commit_readback_verified_binding(source_a.path(), &binding_a).expect("source A commit");
        commit_readback_verified_binding(source_a.path(), &binding_a).expect("source A replay");
        commit_readback_verified_binding(source_b.path(), &binding_b).expect("source B commit");

        let projection =
            crate::cli::improvement_contract::read_owner_projection().expect("revisioned reader");
        assert_eq!(projection.owners.len(), 1);
        assert_eq!(projection.owners[0].aggregate_count, 2);
        assert_eq!(projection.owners[0].occurrences.len(), 2);

        let raw_path = crate::cli::improvement_store::owner_projection_path();
        let raw = fs::read_to_string(raw_path).expect("projection JSON");
        for forbidden in [
            "1".repeat(64),
            "2".repeat(64),
            source_a.path().display().to_string(),
            source_b.path().display().to_string(),
            "Private candidate".to_string(),
            "customer/private-target".to_string(),
        ] {
            assert!(
                !raw.contains(&forbidden),
                "projection leaked {forbidden}: {raw}"
            );
        }

        for source in [source_a.path(), source_b.path()] {
            let store = crate::cli::improvement_store::load_and_repair(source)
                .expect("source store remains readable");
            assert!(store.candidates[0].owner.is_none());
            assert_eq!(store.candidates[0].state, CandidateState::OwnerResolving);
        }
    }

    #[test]
    fn projection_commit_rejects_conflicting_opaque_key_and_failure_before_persist() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source_a = tempfile::tempdir().expect("source A");
        let source_b = tempfile::tempdir().expect("source B");
        let nonce = "3".repeat(64);
        let (fingerprint, occurrence_key) = store_projection_candidate(
            source_a.path(),
            &nonce,
            "impr-source-a",
            "conflicting-event",
        );
        let (_, conflicting_key) = store_projection_candidate(
            source_b.path(),
            &nonce,
            "impr-source-b",
            "conflicting-event",
        );
        assert_eq!(occurrence_key, conflicting_key);
        let binding_a = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-source-a".to_string(),
            owner: verified_projection_owner(41, &fingerprint),
            occurrence_key,
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T08:01:00Z".to_string(),
        };
        let binding_b = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-source-b".to_string(),
            owner: verified_projection_owner(42, &fingerprint),
            occurrence_key: conflicting_key,
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T08:02:00Z".to_string(),
        };

        let interrupted = commit_readback_verified_binding_for_test(
            source_a.path(),
            &binding_a,
            ProjectionCommitFailurePoint::BeforePersist,
        );
        assert!(interrupted.is_err());
        assert!(
            !crate::cli::improvement_store::owner_projection_path().exists(),
            "failure before projection commit must not create a false owner"
        );

        commit_readback_verified_binding(source_a.path(), &binding_a).expect("first commit");
        let error = commit_readback_verified_binding(source_b.path(), &binding_b)
            .expect_err("conflicting opaque occurrence must fail closed");
        assert!(error
            .to_string()
            .contains("conflicting owner projection occurrence"));
        let projection = crate::cli::improvement_contract::read_owner_projection()
            .expect("projection after conflict");
        assert_eq!(projection.owners.len(), 1);
        assert_eq!(projection.owners[0].aggregate_count, 1);
    }

    #[test]
    fn projection_commit_rejects_private_source_data_in_owner_title() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("private source");
        let (fingerprint, occurrence_key) = store_projection_candidate(
            source.path(),
            &"6".repeat(64),
            "impr-private-owner-title",
            "private-title-event",
        );
        let mut owner = verified_projection_owner(77, &fingerprint);
        owner.title = format!("Failure in {}", source.path().display());
        let binding = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-private-owner-title".to_string(),
            owner,
            occurrence_key,
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T08:00:00Z".to_string(),
        };

        let error = commit_readback_verified_binding(source.path(), &binding)
            .expect_err("private owner title must not enter the projection");
        assert!(error.to_string().contains("privacy"));
        assert!(!crate::cli::improvement_store::owner_projection_path().exists());
    }

    #[test]
    fn live_projection_commit_rejects_migration_only_unknown_comment_audit() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let (fingerprint, occurrence_key) = store_projection_candidate(
            source.path(),
            &"7".repeat(64),
            "impr-unverified-comment-audit",
            "unverified-comment-event",
        );
        let binding = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-unverified-comment-audit".to_string(),
            owner: verified_projection_owner(79, &fingerprint),
            occurrence_key,
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T08:00:00Z".to_string(),
        };
        let commit = prepare_owner_projection_commit(source.path(), &binding)
            .expect("prepare unverified projection commit");

        let error = crate::cli::improvement_store::commit_owner_projection(commit)
            .expect_err("live commit must reject migration-only audit uncertainty");

        assert!(error.to_string().contains("legacy-unknown"));
        assert!(!crate::cli::improvement_store::owner_projection_path().exists());
    }

    #[test]
    fn projection_reconciliation_moves_an_occurrence_to_the_lowest_owner_without_downgrade() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let (fingerprint, occurrence_key) = store_projection_candidate(
            source.path(),
            &"4".repeat(64),
            "impr-reconciliation",
            "reconciliation-event",
        );
        let losing = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-reconciliation".to_string(),
            owner: verified_projection_owner(42, &fingerprint),
            occurrence_key: occurrence_key.clone(),
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T08:00:00Z".to_string(),
        };
        let canonical = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-reconciliation".to_string(),
            owner: verified_projection_owner(41, &fingerprint),
            occurrence_key,
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T08:01:00Z".to_string(),
        };

        commit_binding_with_complete_comment_audit(source.path(), &losing, &[10, 11])
            .expect("losing owner commit");
        commit_binding_with_complete_comment_audit(source.path(), &canonical, &[20])
            .expect("lowest owner reconciliation commit");
        commit_binding_with_complete_comment_audit(source.path(), &losing, &[12])
            .expect("delayed losing owner audit must join without reclaiming the occurrence");
        repair_source_success_snapshots(source.path()).expect("repair canonical owner");

        let projection = crate::cli::improvement_contract::read_owner_projection()
            .expect("projection after reconciliation");
        assert_eq!(projection.owners.len(), 1);
        assert_eq!(projection.owners[0].owner.number, 41);
        let source_store =
            crate::cli::improvement_store::load_and_repair(source.path()).expect("repaired source");
        assert_eq!(source_store.candidates[0].state, CandidateState::Linked);
        assert_eq!(
            source_store.candidates[0].owner.as_ref().unwrap().number,
            41
        );
        let private_projection: serde_json::Value = serde_json::from_slice(
            &fs::read(crate::cli::improvement_store::owner_projection_path())
                .expect("private projection storage"),
        )
        .expect("parse private projection storage");
        assert_eq!(
            private_projection["owners"][0]["occurrences"][0]["comment_audit"],
            json!({
                "completeness": "complete",
                "physical_comments": [
                    {"owner_number": 41, "comment_id": 20},
                    {"owner_number": 42, "comment_id": 10},
                    {"owner_number": 42, "comment_id": 11},
                    {"owner_number": 42, "comment_id": 12}
                ]
            })
        );

        let stale = commit_readback_verified_binding(source.path(), &losing)
            .expect_err("higher losing owner must not reclaim the occurrence");
        assert!(stale.to_string().contains("canonical owner"));
    }

    #[test]
    fn created_canonical_owner_preserves_historical_link_comment_audit_in_both_orders() {
        for low_owner_first in [true, false] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let source = tempfile::tempdir().expect("source");
            let (fingerprint, occurrence_key) = store_projection_candidate(
                source.path(),
                &"8".repeat(64),
                "impr-created-reconciliation",
                "created-reconciliation-event",
            );
            let canonical_created = ReadbackVerifiedOwnerBinding {
                candidate_id: "impr-created-reconciliation".to_string(),
                owner: verified_projection_owner(41, &fingerprint),
                occurrence_key: occurrence_key.clone(),
                resolution_status: CandidateState::Created,
                last_seen: "2026-07-15T08:01:00Z".to_string(),
            };
            let historical_link = ReadbackVerifiedOwnerBinding {
                candidate_id: "impr-created-reconciliation".to_string(),
                owner: verified_projection_owner(42, &fingerprint),
                occurrence_key,
                resolution_status: CandidateState::Linked,
                last_seen: "2026-07-15T08:00:00Z".to_string(),
            };

            if low_owner_first {
                commit_readback_verified_binding(source.path(), &canonical_created)
                    .expect("canonical created owner");
                commit_binding_with_complete_comment_audit(source.path(), &historical_link, &[20])
                    .expect("delayed historical link audit");
            } else {
                commit_binding_with_complete_comment_audit(source.path(), &historical_link, &[20])
                    .expect("historical link audit");
                commit_readback_verified_binding(source.path(), &canonical_created)
                    .expect("lower created owner reconciliation");
            }
            repair_source_success_snapshots(source.path()).expect("repair canonical owner");

            let projection = crate::cli::improvement_contract::read_owner_projection()
                .expect("projection after created reconciliation");
            assert_eq!(projection.owners[0].owner.number, 41);
            let private_projection: serde_json::Value = serde_json::from_slice(
                &fs::read(crate::cli::improvement_store::owner_projection_path())
                    .expect("private projection storage"),
            )
            .expect("parse private projection storage");
            assert_eq!(
                private_projection["owners"][0]["occurrences"][0]["comment_audit"],
                json!({
                    "completeness": "complete",
                    "physical_comments": [
                        {"owner_number": 42, "comment_id": 20}
                    ]
                })
            );
            let source_store = crate::cli::improvement_store::load_and_repair(source.path())
                .expect("repaired source");
            assert_eq!(source_store.candidates[0].state, CandidateState::Created);
            assert_eq!(
                source_store.candidates[0].owner.as_ref().unwrap().number,
                41
            );
        }
    }

    #[test]
    fn projection_repair_uses_union_coverage_and_preserves_created_disposition() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let source = tempfile::tempdir().expect("source");
        let nonce = "5".repeat(64);
        let (fingerprint, first_occurrence) =
            store_projection_candidate(source.path(), &nonce, "impr-regression", "original-event");
        let recurrent_occurrence =
            append_projection_occurrence(source.path(), &nonce, "regression-event");
        let historical = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-regression".to_string(),
            owner: verified_projection_owner(10, &fingerprint),
            occurrence_key: first_occurrence,
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T08:00:00Z".to_string(),
        };
        let regression = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-regression".to_string(),
            owner: verified_projection_owner(20, &fingerprint),
            occurrence_key: recurrent_occurrence,
            resolution_status: CandidateState::Created,
            last_seen: "2026-07-15T09:01:00Z".to_string(),
        };

        commit_readback_verified_binding(source.path(), &historical)
            .expect("historical owner commit");
        commit_readback_verified_binding(source.path(), &regression)
            .expect("regression owner commit");
        let replay_as_linked = ReadbackVerifiedOwnerBinding {
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T09:02:00Z".to_string(),
            ..regression.clone()
        };
        commit_readback_verified_binding(source.path(), &replay_as_linked)
            .expect("later replay must not downgrade created disposition");
        assert!(repair_source_success_snapshots(source.path()).expect("repair regression owner"));

        let source_store =
            crate::cli::improvement_store::load_and_repair(source.path()).expect("repaired source");
        let candidate = &source_store.candidates[0];
        assert_eq!(candidate.state, CandidateState::Created);
        assert_eq!(candidate.owner.as_ref().unwrap().number, 20);
    }
}
