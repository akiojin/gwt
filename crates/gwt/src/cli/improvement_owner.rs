#![cfg_attr(not(test), allow(dead_code))]

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
    sync::OnceLock,
    time::{Duration, Instant},
};

use chrono::Utc;
use gwt_github::client::{
    ApiError as GitHubApiError, IssueNumber, IssueState, OwnerRepositoryClient, RepositoryIdentity,
    RepositoryIssue, RepositoryIssueKind, ResolutionDeadline,
};
use regex::Regex;
use sha2::{Digest, Sha256};

use super::{
    improvement::{
        improvement_fingerprint, post_improvement_board_status, transition_candidate,
        typed_evidence_digest, BlockedReason, CandidateState, DurableOwnerSnapshot, FailureSubcode,
        ImprovementCandidate, LinkedIssue, OccurrenceOrigin, OccurrenceReplayProof, OwnerCandidate,
        OwnerKind, OwnerMatchBasis, ResolverSnapshot, RetryMetadata, TypedFailureEvidence,
    },
    improvement_contract::{OwnerProjectionOwner, OwnerProjectionOwnerKind},
    improvement_store::{
        OwnerProjectionCommit, OwnerProjectionRecord, OwnerProjectionResolutionStatus,
        OwnerProjectionSourceReference, OwnerProjectionStore,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ContractOwnerMapping {
    pub(super) contract_id: String,
    pub(super) contract_schema_revision: u64,
    pub(super) routing_basis_revision: u64,
    pub(super) owner_number: IssueNumber,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ContractRoutingRegistry {
    mappings: Vec<ContractOwnerMapping>,
}

impl ContractRoutingRegistry {
    pub(super) fn new(mappings: Vec<ContractOwnerMapping>) -> Self {
        Self { mappings }
    }

    fn matching_owner_numbers(&self, candidate: &ImprovementCandidate) -> Vec<IssueNumber> {
        let Some(evidence) = candidate.typed_evidence.as_ref() else {
            return Vec::new();
        };
        self.mappings
            .iter()
            .filter(|mapping| {
                mapping.contract_id == evidence.contract_id
                    && mapping.contract_schema_revision == evidence.contract_schema_revision
                    && candidate.distinct_occurrences.iter().any(|occurrence| {
                        occurrence.origin == OccurrenceOrigin::Deterministic
                            && occurrence.qualifies_unattended
                            && occurrence.producer_registry_revision.is_some()
                            && occurrence.routing_basis_revision
                                == Some(mapping.routing_basis_revision)
                    })
            })
            .map(|mapping| mapping.owner_number)
            .collect()
    }
}

fn commit_readback_verified_binding(
    source_repo_root: &Path,
    binding: &ReadbackVerifiedOwnerBinding,
) -> Result<(), gwt_github::SpecOpsError> {
    let commit = prepare_owner_projection_commit(source_repo_root, binding)?;
    super::improvement_store::commit_owner_projection(commit)
}

#[cfg(test)]
fn commit_readback_verified_binding_for_test(
    source_repo_root: &Path,
    binding: &ReadbackVerifiedOwnerBinding,
    failure_point: ProjectionCommitFailurePoint,
) -> Result<(), gwt_github::SpecOpsError> {
    let commit = prepare_owner_projection_commit(source_repo_root, binding)?;
    match failure_point {
        ProjectionCommitFailurePoint::BeforePersist => {
            super::improvement_store::commit_owner_projection_before_persist(commit)
        }
    }
}

fn prepare_owner_projection_commit(
    source_repo_root: &Path,
    binding: &ReadbackVerifiedOwnerBinding,
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
    validate_projection_owner_snapshot(&binding.owner)?;
    let projection_privacy_context =
        PublicMutationContext::for_repo(source_repo_root).with_candidate(candidate, &[]);
    validate_public_payload(
        &PublicIssuePayload {
            summary: "Verified upstream owner identity".to_string(),
            title: binding.owner.title.clone(),
            body: "Verified upstream owner identity.".to_string(),
        },
        &projection_privacy_context,
    )
    .map_err(|error| {
        owner_projection_error(&format!(
            "owner projection privacy validation failed: {error}"
        ))
    })?;
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
        occurrence: super::improvement_contract::OwnerProjectionOccurrence {
            opaque_key: binding.occurrence_key.clone(),
            public_marker_digest,
            last_seen: binding.last_seen.clone(),
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
        for candidate in &mut store.candidates {
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

fn source_store_needs_projection_repair(
    store: &super::improvement::CandidateStore,
    projection: &OwnerProjectionStore,
) -> Result<bool, gwt_github::SpecOpsError> {
    let source_scope_nonce = store
        .source_scope_nonce
        .as_deref()
        .ok_or_else(|| owner_projection_error("source scope nonce is missing"))?;
    for candidate in &store.candidates {
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
    candidate.attempt = None;
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
        corpus_generation: String,
    },
    Historical {
        owners: Vec<OwnerCandidate>,
        corpus_generation: String,
    },
    Zero {
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

#[derive(Debug)]
enum OwnerInspection {
    Active {
        owner: OwnerCandidate,
        corpus_generation: String,
    },
    Historical {
        owners: Vec<OwnerCandidate>,
        corpus_generation: String,
    },
    Zero {
        corpus_generation: String,
        advisory_failed: bool,
    },
    Ambiguous {
        owner_candidates: Vec<OwnerCandidate>,
        corpus_generation: String,
    },
}

#[derive(Debug, Clone, Copy)]
struct OwnerResolutionFailure {
    reason: BlockedReason,
    failure_subcode: Option<FailureSubcode>,
    remediation: &'static str,
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
    if deadline.remaining("collect owner privacy context").is_err() {
        let failure = OwnerResolutionFailure {
            reason: BlockedReason::Timeout,
            failure_subcode: None,
            remediation: "RETRY_OWNER_RESOLUTION",
        };
        block_owner_resolution(env, candidate, failure, None)?;
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
            block_owner_resolution(env, candidate, failure, None)?;
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
        block_owner_resolution(env, candidate, failure, None)?;
        return Ok(owner_failure_outcome(candidate, failure));
    }
    if let Some(failure) = candidate_preflight_failure(
        candidate,
        &repo_path,
        &canonical_source_scope_nonce,
        &public_context,
    ) {
        block_owner_resolution(env, candidate, failure, None)?;
        return Ok(owner_failure_outcome(candidate, failure));
    }

    let inspection = match env.improvement_owner_client(deadline) {
        Ok(client) => inspect_owner_corpus(client, candidate, registry, semantic_advisor, deadline),
        Err(error) => Err(owner_failure_from_api(&error)),
    };

    match inspection {
        Ok(OwnerInspection::Active {
            owner,
            corpus_generation,
        }) => Ok(OwnerPreflightOutcome::Active {
            owner,
            corpus_generation,
        }),
        Ok(OwnerInspection::Historical {
            owners,
            corpus_generation,
        }) => Ok(OwnerPreflightOutcome::Historical {
            owners,
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
            Ok(OwnerPreflightOutcome::Zero { corpus_generation })
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
            block_owner_resolution(env, candidate, failure, Some(snapshot))?;
            Ok(owner_failure_outcome(candidate, failure))
        }
        Err(failure) => {
            block_owner_resolution(env, candidate, failure, None)?;
            Ok(owner_failure_outcome(candidate, failure))
        }
    }
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
    let occurrence_payloads_are_safe = candidate
        .distinct_occurrences
        .iter()
        .filter(|occurrence| occurrence.qualifies_unattended)
        .all(|occurrence| {
            render_occurrence_comment_payload(candidate, &occurrence.opaque_key, public_context)
                .is_ok()
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

    let owner_comment_generation = if matches.len() == 1 {
        let owner_number = *matches.keys().next().expect("one authoritative owner");
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
        Some(format!(
            "{}:{}",
            owner_number.0,
            comments.generation().as_str()
        ))
    } else {
        None
    };

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
    generation_fields.extend(owner_comment_generation);
    let corpus_generation = combined_corpus_generation(generation_fields);

    if !matches.is_empty() {
        let mut owners = matches.into_values().collect::<Vec<_>>();
        if owners.len() != 1 {
            return Ok(OwnerInspection::Ambiguous {
                owner_candidates: owners,
                corpus_generation,
            });
        }

        let owner = owners.pop().expect("one authoritative owner");
        if owner.active {
            return Ok(OwnerInspection::Active {
                owner,
                corpus_generation,
            });
        }
        return Ok(OwnerInspection::Historical {
            owners: vec![owner],
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

fn block_owner_resolution<E: CliEnv>(
    env: &mut E,
    candidate: &mut ImprovementCandidate,
    failure: OwnerResolutionFailure,
    resolver_snapshot: Option<ResolverSnapshot>,
) -> Result<(), gwt_github::SpecOpsError> {
    if candidate.state == CandidateState::RemoteOutcomeUnknown {
        candidate.retry = Some(RetryMetadata {
            retryable: true,
            remediation: "REFRESH_OWNER_CORPUS".to_string(),
            failed_at: Utc::now().to_rfc3339(),
        });
        candidate.resolver_snapshot = resolver_snapshot;
        candidate.updated_at = Utc::now().to_rfc3339();
        return post_remote_outcome_unknown_status(env, candidate, failure);
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
    post_owner_resolution_blocked_status(env, candidate, failure)
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
        "## Problem\n\n{problem}\n\n## Expected behavior\n\n{expected}\n\n## Observed evidence\n\n{observed}\n\n{identity}\n## Impact\n\nA gwt-owned contract failure can recur until it has one verified upstream owner.\n\n## Suggested verification\n\n- Reproduce the typed contract outcome.\n- Add a regression test that fails before the fix.\n- Verify the corrected outcome without private source data.\n\n## Source candidate\n\n- Candidate ID: {id}\n- Target artifact: {target}\n- Classification: {classification}\n- Confidence: {confidence}\n- Occurrences: {occurrences}\n\n## Privacy\n\n- This payload is generated from contract-owned typed fields.\n- Free-form evidence, repository identity, paths, credentials, logs, and code remain local-only.\n",
        id = candidate.id,
        target = candidate.target_artifact,
        classification = candidate.classification,
        confidence = candidate.confidence,
        occurrences = candidate.occurrences,
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
            fingerprint_marker_re()
                .captures(line)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().to_string())
        })
        .collect()
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
        gwt_core::process_console::SpawnOptions::new("git remote origin"),
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
            opaque_occurrence_key, CandidateState, DistinctOccurrence, DurableOwnerSnapshot,
            ImprovementCandidate, ImprovementEligibility, OccurrenceOrigin, OccurrenceReplayProof,
            OwnerKind, OwnerMatchBasis,
        },
        run_collect, BoardCommand, CliCommand, TestEnv,
    };
    use gwt_github::client::{
        fake::{FakeIssueClient, OwnerRepositoryFaultTiming, OwnerRepositoryOperation},
        ApiError as GitHubApiError, CommentId, IssueNumber, IssueState, RepositoryComment,
        RepositoryIdentity, RepositoryIssue, RepositoryIssueKind, ResolutionDeadline, UpdatedAt,
    };

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
            .map(|id| RepositoryComment {
                id: CommentId(id),
                body: format!("public comment {id}"),
                updated_at: UpdatedAt::new(format!("c{id}")),
            })
            .collect::<Vec<_>>();
        comments.push(RepositoryComment {
            id: CommentId(205),
            body: occurrence_marker(&candidate.distinct_occurrences[0].opaque_key),
            updated_at: UpdatedAt::new("c205"),
        });
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
            &ContractRoutingRegistry::default(),
            &StaticSemanticAdvisor(Ok(Vec::new())),
            &resolution_deadline(),
        )
        .expect("preflight");

        assert!(matches!(
            outcome,
            OwnerPreflightOutcome::Zero { corpus_generation }
                if !corpus_generation.is_empty()
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
            &ContractRoutingRegistry::default(),
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
                    env.owner_client.seed_repository_issue(owner_issue(
                        42,
                        IssueState::Open,
                        marker.clone(),
                    ));
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
            &ContractRoutingRegistry::default(),
            &StaticSemanticAdvisor(Err(())),
            &resolution_deadline(),
        )
        .expect("preflight");
        assert!(matches!(outcome, OwnerPreflightOutcome::Zero { .. }));
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
            &ContractRoutingRegistry::default(),
            &advisor,
            &deadline,
        )
        .expect("preflight");

        assert!(matches!(outcome, OwnerPreflightOutcome::Zero { .. }));
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
        });
        candidate.occurrences = 2;
        let registry = ContractRoutingRegistry::new(vec![ContractOwnerMapping {
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            routing_basis_revision: 9,
            owner_number: IssueNumber(77),
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
        });
        candidate.occurrences = 2;
        let registry = ContractRoutingRegistry::new(vec![ContractOwnerMapping {
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            routing_basis_revision: 9,
            owner_number: IssueNumber(77),
        }]);

        let outcome = owner_resolution_preflight(
            &mut env,
            &mut candidate,
            &registry,
            &StaticSemanticAdvisor(Ok(Vec::new())),
            &resolution_deadline(),
        )
        .expect("preflight");

        assert!(matches!(outcome, OwnerPreflightOutcome::Zero { .. }));
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
            resolution_status: CandidateState::Created,
            last_seen: "2026-07-15T08:00:00Z".to_string(),
        };
        let canonical = ReadbackVerifiedOwnerBinding {
            candidate_id: "impr-reconciliation".to_string(),
            owner: verified_projection_owner(41, &fingerprint),
            occurrence_key,
            resolution_status: CandidateState::Linked,
            last_seen: "2026-07-15T08:01:00Z".to_string(),
        };

        commit_readback_verified_binding(source.path(), &losing).expect("losing owner commit");
        commit_readback_verified_binding(source.path(), &canonical)
            .expect("lowest owner reconciliation commit");
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

        let stale = commit_readback_verified_binding(source.path(), &losing)
            .expect_err("higher losing owner must not reclaim the occurrence");
        assert!(stale.to_string().contains("canonical owner"));
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
