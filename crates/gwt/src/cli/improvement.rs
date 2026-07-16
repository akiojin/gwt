use std::{collections::BTreeSet, fs, path::Path, str::FromStr};

use chrono::Utc;
use gwt_github::{client::ApiError, SpecOpsError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    improvement_owner::{
        deliver_pending_owner_status, render_public_issue_payload, resolve_candidate_owner,
        PublicMutationContext,
    },
    CliEnv,
};

const UPSTREAM_REPOSITORY: &str = "akiojin/gwt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImprovementCommand {
    Capture(Box<ImprovementCaptureCommand>),
    List(ImprovementListCommand),
    Dismiss(ImprovementDismissCommand),
    LinkIssue(ImprovementLinkIssueCommand),
    PromoteIssue(ImprovementPromoteIssueCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementCaptureCommand {
    pub source: String,
    pub target_artifact: String,
    pub classification: String,
    pub confidence: String,
    pub summary: String,
    pub details: Option<String>,
    pub evidence_digest: Option<String>,
    pub dedupe_key: Option<String>,
    pub local_evidence: Vec<Value>,
    pub typed_evidence: Option<ImprovementTypedEvidenceCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementTypedEvidenceCommand {
    pub subsystem: String,
    pub contract_id: String,
    pub contract_schema_revision: u64,
    pub failure_code: String,
    pub expected_outcome: String,
    pub observed_outcome: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateState {
    Pending,
    NeedsEvidence,
    OwnerResolving,
    Linked,
    #[serde(alias = "promoted")]
    Created,
    Blocked,
    RemoteOutcomeUnknown,
    Recurrent,
    Parked,
    Dismissed,
}

impl CandidateState {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::NeedsEvidence => "needs-evidence",
            Self::OwnerResolving => "owner-resolving",
            Self::Linked => "linked",
            Self::Created => "created",
            Self::Blocked => "blocked",
            Self::RemoteOutcomeUnknown => "remote-outcome-unknown",
            Self::Recurrent => "recurrent",
            Self::Parked => "parked",
            Self::Dismissed => "dismissed",
        }
    }

    const fn compatibility_state(self) -> &'static str {
        match self {
            Self::Created => "promoted",
            other => other.as_str(),
        }
    }
}

impl FromStr for CandidateState {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "needs-evidence" => Ok(Self::NeedsEvidence),
            "owner-resolving" => Ok(Self::OwnerResolving),
            "linked" => Ok(Self::Linked),
            "created" | "promoted" => Ok(Self::Created),
            "blocked" => Ok(Self::Blocked),
            "remote-outcome-unknown" => Ok(Self::RemoteOutcomeUnknown),
            "recurrent" => Ok(Self::Recurrent),
            "parked" => Ok(Self::Parked),
            "dismissed" => Ok(Self::Dismissed),
            _ => Err("invalid improvement state"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BlockedReason {
    Store,
    Search,
    Auth,
    Privacy,
    Ambiguity,
    Routing,
    Create,
    Update,
    Readback,
    LocalCommit,
    Timeout,
    RateLimit,
    Network,
    Parse,
    Reconciliation,
}

impl FromStr for BlockedReason {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "store" => Ok(Self::Store),
            "search" => Ok(Self::Search),
            "auth" => Ok(Self::Auth),
            "privacy" => Ok(Self::Privacy),
            "ambiguity" => Ok(Self::Ambiguity),
            "routing" => Ok(Self::Routing),
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            "readback" => Ok(Self::Readback),
            "local-commit" => Ok(Self::LocalCommit),
            "timeout" => Ok(Self::Timeout),
            "rate-limit" => Ok(Self::RateLimit),
            "network" => Ok(Self::Network),
            "parse" => Ok(Self::Parse),
            "reconciliation" => Ok(Self::Reconciliation),
            _ => Err("invalid blocked reason"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FailureSubcode {
    EmptyCorpus,
    PartialPage,
}

impl FromStr for FailureSubcode {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "empty-corpus" => Ok(Self::EmptyCorpus),
            "partial-page" => Ok(Self::PartialPage),
            _ => Err("invalid failure subcode"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct RetryMetadata {
    pub(super) retryable: bool,
    pub(super) remediation: String,
    pub(super) failed_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum OwnerKind {
    Issue,
    Spec,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum OwnerMatchBasis {
    Fingerprint,
    Contract,
    Semantic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct OwnerCandidate {
    pub(super) number: u64,
    pub(super) kind: OwnerKind,
    pub(super) title: String,
    pub(super) active: bool,
    pub(super) url: String,
    pub(super) match_basis: OwnerMatchBasis,
    pub(super) selectable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct DurableOwnerSnapshot {
    pub(super) number: u64,
    pub(super) kind: OwnerKind,
    pub(super) title: String,
    pub(super) active: bool,
    pub(super) url: String,
    pub(super) fingerprint: String,
    pub(super) readback_verified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ResolverSnapshot {
    pub(super) corpus_generation: String,
    pub(super) resolver_revision: String,
    pub(super) owner_candidates: Vec<OwnerCandidate>,
}

impl ResolverSnapshot {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn new(
        corpus_generation: String,
        owner_candidates: Vec<OwnerCandidate>,
    ) -> Result<Self, SpecOpsError> {
        if corpus_generation.trim().is_empty() {
            return Err(invalid("resolver corpus generation must not be empty"));
        }
        for owner in &owner_candidates {
            validate_owner_candidate(owner)?;
        }
        let resolver_revision = resolver_revision(&corpus_generation, &owner_candidates)?;
        Ok(Self {
            corpus_generation,
            resolver_revision,
            owner_candidates,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementListCommand {
    pub state: Option<CandidateState>,
    pub blocked_reason: Option<BlockedReason>,
    pub failure_subcode: Option<FailureSubcode>,
    pub classification: Option<String>,
    pub confidence: Option<String>,
    pub owner_number: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementDismissCommand {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementLinkIssueCommand {
    pub id: String,
    pub number: u64,
    pub url: Option<String>,
    pub repository: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementPromoteIssueCommand {
    pub id: String,
    pub force: bool,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum ImprovementAuditEntry {
    ManualOwnerSelection {
        owner_number: u64,
        resolver_revision: String,
        corpus_generation: String,
        recorded_at: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CandidateStore {
    #[serde(default)]
    pub(super) schema_version: u32,
    #[serde(default)]
    pub(super) source_scope_nonce: Option<String>,
    #[serde(default)]
    pub(super) candidates: Vec<ImprovementCandidate>,
    #[serde(default)]
    pub(super) legacy_import: super::improvement_store::LegacyImportState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ImprovementCandidate {
    #[serde(default)]
    pub(super) schema_version: u32,
    pub(super) id: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) source: String,
    pub(super) target_artifact: String,
    pub(super) classification: String,
    pub(super) confidence: String,
    pub(super) state: CandidateState,
    #[serde(default)]
    pub(super) blocked_reason: Option<BlockedReason>,
    #[serde(default)]
    pub(super) failure_subcode: Option<FailureSubcode>,
    #[serde(default)]
    pub(super) retry: Option<RetryMetadata>,
    #[serde(default)]
    pub(super) owner: Option<DurableOwnerSnapshot>,
    #[serde(default)]
    pub(super) resolver_snapshot: Option<ResolverSnapshot>,
    pub(super) dedupe_key: String,
    pub(super) occurrences: u64,
    #[serde(default)]
    pub(super) legacy_occurrence_count: Option<u64>,
    #[serde(default)]
    pub(super) fingerprint: Option<String>,
    #[serde(default)]
    pub(super) eligibility: ImprovementEligibility,
    #[serde(default)]
    pub(super) typed_evidence: Option<TypedFailureEvidence>,
    #[serde(default)]
    pub(super) distinct_occurrences: Vec<DistinctOccurrence>,
    #[serde(default)]
    pub(super) capture_status_generation: u64,
    #[serde(default)]
    pub(super) capture_status_delivered_generation: u64,
    #[serde(default)]
    pub(super) owner_status_generation: u64,
    #[serde(default)]
    pub(super) owner_status_delivered_generation: u64,
    #[serde(default)]
    pub(super) reconciliation_required: bool,
    #[serde(default)]
    pub(super) reconciliation_owner_numbers: Vec<u64>,
    pub(super) sanitized_summary: String,
    pub(super) sanitized_details: Option<String>,
    pub(super) evidence_digest: Option<String>,
    pub(super) local_evidence: Vec<LocalEvidenceReference>,
    pub(super) linked_issue: Option<LinkedIssue>,
    pub(super) dismissed_reason: Option<String>,
    #[serde(default)]
    pub(super) legacy_provenance: Vec<super::improvement_store::LegacyProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) pending_create_resolution: Option<super::improvement_store::ResolutionAttemptIntent>,
    #[serde(default)]
    pub(super) attempt: Option<super::improvement_store::ResolutionAttemptLease>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) audit: Vec<ImprovementAuditEntry>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ImprovementEligibility {
    Deterministic,
    InterpretiveCorroboration,
    #[default]
    NeedsEvidence,
    Ineligible,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct TypedFailureEvidence {
    pub(super) subsystem: String,
    pub(super) contract_id: String,
    pub(super) contract_schema_revision: u64,
    pub(super) failure_code: String,
    pub(super) target_artifact: String,
    pub(super) expected_outcome: String,
    pub(super) observed_outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TypedRecurrenceEvidence {
    pub(super) installed_version: Option<String>,
    pub(super) build_commit: Option<String>,
    pub(super) observed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct DistinctOccurrence {
    pub(super) opaque_key: String,
    pub(super) evidence_digest: String,
    pub(super) captured_at: String,
    pub(super) origin: OccurrenceOrigin,
    #[serde(default)]
    pub(super) qualifies_unattended: bool,
    #[serde(default)]
    pub(super) producer_id: Option<String>,
    #[serde(default)]
    pub(super) producer_registry_revision: Option<u64>,
    #[serde(default)]
    pub(super) routing_basis_revision: Option<u64>,
    #[serde(default)]
    pub(super) replay_proof: Option<OccurrenceReplayProof>,
    #[serde(default)]
    pub(super) recurrence: Option<TypedRecurrenceEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum OccurrenceReplayProof {
    InterpretiveSession {
        source_scope_nonce: String,
        session_id: String,
    },
    RegisteredEvent {
        source_scope_nonce: String,
        source_event_id: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum OccurrenceOrigin {
    Interpretive,
    Deterministic,
}

// SPEC-3164 ships the sealed producer contract before #3248 registers the
// first production caller. Keep the temporary allowance scoped to that API.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureBudgetProfile {
    Normal,
    StrictStop,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RegisteredProducerToken {
    registry_revision: u64,
    registration_index: usize,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone)]
pub(crate) struct RegisteredCaptureInput {
    pub(crate) token: RegisteredProducerToken,
    pub(crate) source_event_id: String,
    pub(crate) routing_basis_revision: u64,
    pub(crate) budget_profile: CaptureBudgetProfile,
    pub(crate) source: String,
    pub(crate) target_artifact: String,
    pub(crate) classification: String,
    pub(crate) confidence: String,
    pub(crate) failure_code: String,
    pub(crate) expected_outcome: String,
    pub(crate) observed_outcome: String,
    pub(crate) summary: Option<String>,
    pub(crate) details: Option<String>,
    pub(crate) local_evidence: Vec<Value>,
    pub(crate) recurrence: Option<TypedRecurrenceEvidence>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
struct ProducerRegistration {
    public_id: &'static str,
    producer_id: &'static str,
    subsystem: &'static str,
    contract_id: &'static str,
    contract_schema_revision: u64,
    routing_basis_revision: u64,
    target_artifact: &'static str,
    allowed_budget: CaptureBudgetProfile,
    recurrence_capable: bool,
}

#[cfg_attr(not(test), allow(dead_code))]
const PRODUCER_REGISTRY_REVISION: u64 = 1;
const _: () = assert!(PRODUCER_REGISTRY_REVISION > 0);
#[cfg_attr(not(test), allow(dead_code))]
const REGISTERED_PRODUCERS: &[ProducerRegistration] = &[
    #[cfg(test)]
    ProducerRegistration {
        public_id: "test.coordination-gate",
        producer_id: "test.coordination-gate.v1",
        subsystem: "coordination",
        contract_id: "coordination.board-status",
        contract_schema_revision: 1,
        routing_basis_revision: 1,
        target_artifact: "coordination",
        allowed_budget: CaptureBudgetProfile::Normal,
        recurrence_capable: true,
    },
    #[cfg(test)]
    ProducerRegistration {
        public_id: "test.owner-route",
        producer_id: "test.owner-route.v1",
        subsystem: "coordination",
        contract_id: "coordination.board-status",
        contract_schema_revision: 1,
        routing_basis_revision: 9,
        target_artifact: "coordination",
        allowed_budget: CaptureBudgetProfile::Normal,
        recurrence_capable: false,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingImprovementStopCandidate {
    pub id: String,
    pub summary: String,
    pub target_artifact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LocalEvidenceReference {
    pub(super) kind: String,
    pub(super) path: Option<String>,
    #[serde(default)]
    pub(super) digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LinkedIssue {
    pub(super) number: u64,
    pub(super) url: String,
    pub(super) repository: String,
}

pub fn run<E: CliEnv>(
    env: &mut E,
    command: ImprovementCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let code = match command {
        ImprovementCommand::Capture(command) => capture(env, *command, out)?,
        ImprovementCommand::List(command) => list(env.repo_path(), command, out)?,
        ImprovementCommand::Dismiss(command) => dismiss(env.repo_path(), command, out)?,
        ImprovementCommand::LinkIssue(command) => link_issue(env.repo_path(), command, out)?,
        ImprovementCommand::PromoteIssue(command) => promote_issue(env, command, out)?,
    };
    Ok(code)
}

pub(crate) fn pending_high_confidence_contract_violations(
    repo_root: &Path,
) -> Vec<PendingImprovementStopCandidate> {
    load_store_with_projection_fallback(repo_root)
        .map(|store| {
            store
                .candidates
                .into_iter()
                .filter(|candidate| {
                    candidate.state == CandidateState::Pending
                        && candidate.classification == "gwt-caused"
                        && candidate.confidence == "high"
                        && is_contract_artifact(&candidate.target_artifact)
                })
                .map(|candidate| PendingImprovementStopCandidate {
                    id: candidate.id,
                    summary: candidate.sanitized_summary,
                    target_artifact: candidate.target_artifact,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn candidate_public_values(repo_root: &Path) -> Vec<Value> {
    let privacy_context = PublicMutationContext::for_repo(repo_root);
    let mut candidates = load_store_with_projection_fallback(repo_root)
        .map(|store| store.candidates)
        .unwrap_or_default();
    candidates.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    candidates
        .into_iter()
        .map(|candidate| candidate_public_json(&candidate, &privacy_context))
        .collect()
}

fn is_contract_artifact(target_artifact: &str) -> bool {
    matches!(
        target_artifact,
        "skill"
            | "AGENTS"
            | "hook"
            | "launch"
            | "index"
            | "verification"
            | "coordination"
            | "issue-spec-workflow"
    )
}

fn capture<E: CliEnv>(
    env: &mut E,
    command: ImprovementCaptureCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    validate_enum(
        "source",
        &command.source,
        &[
            "agent-failure",
            "user-correction",
            "review-feedback",
            "hook-runtime",
            "verification",
            "manual",
        ],
    )?;
    validate_enum(
        "target_artifact",
        &command.target_artifact,
        &[
            "skill",
            "AGENTS",
            "hook",
            "launch",
            "index",
            "verification",
            "coordination",
            "issue-spec-workflow",
            "unknown",
        ],
    )?;
    validate_enum(
        "classification",
        &command.classification,
        &["gwt-caused", "ambiguous", "target-project", "external"],
    )?;
    validate_enum(
        "confidence",
        &command.confidence,
        &["low", "medium", "high"],
    )?;
    let typed_evidence = command
        .typed_evidence
        .as_ref()
        .map(|evidence| validate_typed_evidence(evidence, &command.target_artifact))
        .transpose()?;
    if typed_evidence.is_none() && command.summary.trim().is_empty() {
        return Err(invalid("summary must not be empty"));
    }

    let repo_root = env.repo_path().to_path_buf();
    let result = match typed_evidence {
        Some(evidence) => capture_typed_interpretive(&repo_root, &command, evidence)?,
        None => capture_legacy_compatibility(&repo_root, &command)?,
    };
    let updated_existing = result.updated_existing;
    let mut candidate = result.candidate;
    let pending_generation = pending_capture_status_generation(&candidate);
    if let Some(generation) = pending_generation {
        post_candidate_captured_status(env, &candidate, updated_existing)?;
        acknowledge_capture_status(&repo_root, &candidate.id, generation)?;
        candidate.capture_status_delivered_generation = generation;
    }
    if matches!(
        candidate.state,
        CandidateState::Linked | CandidateState::Created
    ) {
        deliver_pending_owner_status(env, &mut candidate)?;
    }
    if should_resolve_after_capture(&candidate, pending_generation) {
        candidate = resolve_candidate_owner(env, &candidate.id, CaptureBudgetProfile::Normal)?;
    }
    write_json(
        out,
        json!({
            "id": candidate.id,
            "state": candidate.state.compatibility_state(),
            "eligibility": candidate.eligibility,
            "fingerprint": candidate.fingerprint,
            "occurrences": candidate.occurrences,
            "legacy_occurrence_count": candidate.legacy_occurrence_count,
            "updated": updated_existing,
            "improvement_contract_version": 2,
        }),
    )?;
    Ok(0)
}

struct CaptureResult {
    candidate: ImprovementCandidate,
    updated_existing: bool,
}

fn capture_typed_interpretive(
    repo_root: &Path,
    command: &ImprovementCaptureCommand,
    evidence: TypedFailureEvidence,
) -> Result<CaptureResult, SpecOpsError> {
    capture_typed(
        repo_root,
        command,
        evidence,
        ValidatedCaptureOrigin::Interpretive {
            session_id: verified_interpretive_session(repo_root),
        },
    )
}

#[cfg_attr(not(test), allow(dead_code))]
enum ValidatedCaptureOrigin {
    Interpretive {
        session_id: Option<String>,
    },
    Registered {
        producer_id: &'static str,
        source_event_id: String,
        producer_registry_revision: u64,
        routing_basis_revision: u64,
        recurrence: Option<TypedRecurrenceEvidence>,
    },
}

fn capture_typed(
    repo_root: &Path,
    command: &ImprovementCaptureCommand,
    evidence: TypedFailureEvidence,
    origin: ValidatedCaptureOrigin,
) -> Result<CaptureResult, SpecOpsError> {
    let now = Utc::now().to_rfc3339();
    let fingerprint = improvement_fingerprint(&evidence);
    let evidence_digest = typed_evidence_digest(&evidence);
    let qualifies_unattended = capture_claim_qualifies(command);
    let sanitized_summary = if command.summary.trim().is_empty() {
        format!("{}: {}", evidence.contract_id, evidence.failure_code)
    } else {
        sanitize_text(&command.summary)
    };
    super::improvement_store::update(repo_root, |store| {
        let nonce = store
            .source_scope_nonce
            .as_deref()
            .ok_or_else(|| invalid("improvement source scope nonce is missing"))?;
        let occurrence = match &origin {
            ValidatedCaptureOrigin::Interpretive {
                session_id: Some(session_id),
            } => Some(DistinctOccurrence {
                opaque_key: opaque_occurrence_key(
                    nonce,
                    &fingerprint,
                    "json.interpretive",
                    session_id,
                ),
                evidence_digest: evidence_digest.clone(),
                captured_at: now.clone(),
                origin: OccurrenceOrigin::Interpretive,
                qualifies_unattended,
                producer_id: None,
                producer_registry_revision: None,
                routing_basis_revision: None,
                replay_proof: Some(OccurrenceReplayProof::InterpretiveSession {
                    source_scope_nonce: nonce.to_string(),
                    session_id: session_id.clone(),
                }),
                recurrence: None,
            }),
            ValidatedCaptureOrigin::Interpretive { session_id: None } => None,
            ValidatedCaptureOrigin::Registered {
                producer_id,
                source_event_id,
                producer_registry_revision,
                routing_basis_revision,
                recurrence,
            } => Some(DistinctOccurrence {
                opaque_key: opaque_occurrence_key(
                    nonce,
                    &fingerprint,
                    producer_id,
                    source_event_id,
                ),
                evidence_digest: occurrence_evidence_digest(&evidence, recurrence.as_ref()),
                captured_at: now.clone(),
                origin: OccurrenceOrigin::Deterministic,
                qualifies_unattended,
                producer_id: Some((*producer_id).to_string()),
                producer_registry_revision: Some(*producer_registry_revision),
                routing_basis_revision: Some(*routing_basis_revision),
                replay_proof: Some(OccurrenceReplayProof::RegisteredEvent {
                    source_scope_nonce: nonce.to_string(),
                    source_event_id: source_event_id.clone(),
                }),
                recurrence: recurrence.clone(),
            }),
        };

        if let Some(candidate) = store
            .candidates
            .iter_mut()
            .find(|candidate| candidate.fingerprint.as_deref() == Some(fingerprint.as_str()))
        {
            let Some(occurrence) = occurrence else {
                return Ok(CaptureResult {
                    candidate: candidate.clone(),
                    updated_existing: true,
                });
            };
            if let Some(existing) = candidate
                .distinct_occurrences
                .iter()
                .find(|existing| existing.opaque_key == occurrence.opaque_key)
            {
                if !same_occurrence_replay(existing, &occurrence) {
                    return Err(invalid("conflicting improvement occurrence replay"));
                }
                return Ok(CaptureResult {
                    candidate: candidate.clone(),
                    updated_existing: true,
                });
            }

            candidate.distinct_occurrences.push(occurrence);
            candidate.updated_at = now.clone();
            if qualifies_unattended {
                candidate.source = command.source.clone();
                candidate.classification = command.classification.clone();
                candidate.confidence = command.confidence.clone();
                candidate.typed_evidence = Some(evidence.clone());
                candidate.sanitized_summary = sanitized_summary.clone();
                candidate.sanitized_details = command.details.as_deref().map(sanitize_text);
                candidate.evidence_digest = Some(evidence_digest.clone());
                candidate.local_evidence = sanitize_local_evidence(&command.local_evidence);
            }
            candidate.dedupe_key = format!("fingerprint:{fingerprint}");
            candidate.occurrences = candidate.distinct_occurrences.len() as u64;
            candidate.eligibility = typed_eligibility(candidate);
            let next_state = settled_typed_capture_state(&candidate.state, candidate.eligibility);
            transition_candidate(candidate, next_state)?;
            if qualifies_unattended {
                queue_capture_status(candidate)?;
            }
            return Ok(CaptureResult {
                candidate: candidate.clone(),
                updated_existing: true,
            });
        }

        let distinct_occurrences = occurrence.into_iter().collect::<Vec<_>>();
        let mut candidate = ImprovementCandidate {
            schema_version: super::improvement_store::STORE_SCHEMA_VERSION,
            id: format!("impr-{}", Uuid::new_v4().simple()),
            created_at: now.clone(),
            updated_at: now,
            source: command.source.clone(),
            target_artifact: command.target_artifact.clone(),
            classification: command.classification.clone(),
            confidence: command.confidence.clone(),
            state: CandidateState::NeedsEvidence,
            blocked_reason: None,
            failure_subcode: None,
            retry: None,
            owner: None,
            resolver_snapshot: None,
            dedupe_key: format!("fingerprint:{fingerprint}"),
            occurrences: distinct_occurrences.len() as u64,
            legacy_occurrence_count: None,
            fingerprint: Some(fingerprint),
            eligibility: ImprovementEligibility::NeedsEvidence,
            typed_evidence: Some(evidence),
            distinct_occurrences,
            capture_status_generation: u64::from(qualifies_unattended),
            capture_status_delivered_generation: 0,
            owner_status_generation: 0,
            owner_status_delivered_generation: 0,
            reconciliation_required: false,
            reconciliation_owner_numbers: Vec::new(),
            sanitized_summary,
            sanitized_details: command.details.as_deref().map(sanitize_text),
            evidence_digest: Some(evidence_digest),
            local_evidence: sanitize_local_evidence(&command.local_evidence),
            linked_issue: None,
            dismissed_reason: None,
            legacy_provenance: Vec::new(),
            pending_create_resolution: None,
            attempt: None,
            audit: Vec::new(),
        };
        candidate.eligibility = typed_eligibility(&candidate);
        let next_state = settled_typed_capture_state(&candidate.state, candidate.eligibility);
        transition_candidate(&mut candidate, next_state)?;
        store.candidates.push(candidate.clone());
        Ok(CaptureResult {
            candidate,
            updated_existing: false,
        })
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn registered_producer_token(
    public_id: &str,
) -> Result<RegisteredProducerToken, SpecOpsError> {
    let registration_index = REGISTERED_PRODUCERS
        .iter()
        .position(|registration| registration.public_id == public_id)
        .ok_or_else(|| invalid("improvement producer is not registered"))?;
    Ok(RegisteredProducerToken {
        registry_revision: PRODUCER_REGISTRY_REVISION,
        registration_index,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn capture_registered<E: CliEnv>(
    env: &mut E,
    input: RegisteredCaptureInput,
) -> Result<ImprovementCandidate, SpecOpsError> {
    let registration = registration_for_token(input.token)?;
    if input.routing_basis_revision != registration.routing_basis_revision {
        return Err(invalid("registered producer routing revision is stale"));
    }
    if input.budget_profile != registration.allowed_budget {
        return Err(invalid(
            "registered producer budget profile is not allowlisted",
        ));
    }
    if input.target_artifact != registration.target_artifact {
        return Err(invalid(
            "registered producer target artifact does not match its registry entry",
        ));
    }
    if input.recurrence.is_some() && !registration.recurrence_capable {
        return Err(invalid("registered producer is not recurrence-capable"));
    }
    validate_enum(
        "source",
        &input.source,
        &[
            "agent-failure",
            "user-correction",
            "review-feedback",
            "hook-runtime",
            "verification",
            "manual",
        ],
    )?;
    validate_enum(
        "classification",
        &input.classification,
        &["gwt-caused", "ambiguous", "target-project", "external"],
    )?;
    validate_enum("confidence", &input.confidence, &["low", "medium", "high"])?;
    let source_event_id = normalize_lower_token("source_event_id", &input.source_event_id)?;
    let evidence = validate_typed_evidence(
        &ImprovementTypedEvidenceCommand {
            subsystem: registration.subsystem.to_string(),
            contract_id: registration.contract_id.to_string(),
            contract_schema_revision: registration.contract_schema_revision,
            failure_code: input.failure_code.clone(),
            expected_outcome: input.expected_outcome.clone(),
            observed_outcome: input.observed_outcome.clone(),
        },
        registration.target_artifact,
    )?;
    let command = ImprovementCaptureCommand {
        source: input.source,
        target_artifact: input.target_artifact,
        classification: input.classification,
        confidence: input.confidence,
        summary: input.summary.unwrap_or_default(),
        details: input.details,
        evidence_digest: None,
        dedupe_key: None,
        local_evidence: input.local_evidence,
        typed_evidence: None,
    };
    let repo_root = env.repo_path().to_path_buf();
    let result = capture_typed(
        &repo_root,
        &command,
        evidence,
        ValidatedCaptureOrigin::Registered {
            producer_id: registration.producer_id,
            source_event_id,
            producer_registry_revision: PRODUCER_REGISTRY_REVISION,
            routing_basis_revision: registration.routing_basis_revision,
            recurrence: input.recurrence,
        },
    )?;
    let mut candidate = result.candidate;
    let pending_generation = pending_capture_status_generation(&candidate);
    if let Some(generation) = pending_generation {
        post_candidate_captured_status(env, &candidate, result.updated_existing)?;
        acknowledge_capture_status(&repo_root, &candidate.id, generation)?;
        candidate.capture_status_delivered_generation = generation;
    }
    if matches!(
        candidate.state,
        CandidateState::Linked | CandidateState::Created
    ) {
        deliver_pending_owner_status(env, &mut candidate)?;
    }
    if should_resolve_after_capture(&candidate, pending_generation) {
        candidate = resolve_candidate_owner(env, &candidate.id, input.budget_profile)?;
    }
    Ok(candidate)
}

#[cfg_attr(not(test), allow(dead_code))]
fn registration_for_token(
    token: RegisteredProducerToken,
) -> Result<&'static ProducerRegistration, SpecOpsError> {
    if token.registry_revision != PRODUCER_REGISTRY_REVISION {
        return Err(invalid("improvement producer registry revision is invalid"));
    }
    REGISTERED_PRODUCERS
        .get(token.registration_index)
        .ok_or_else(|| invalid("improvement producer token is invalid"))
}

fn capture_legacy_compatibility(
    repo_root: &Path,
    command: &ImprovementCaptureCommand,
) -> Result<CaptureResult, SpecOpsError> {
    let now = Utc::now().to_rfc3339();
    let sanitized_summary = sanitize_text(&command.summary);
    let dedupe_key = command
        .dedupe_key
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| default_dedupe_key(command, &sanitized_summary));
    super::improvement_store::update(repo_root, |store| {
        if let Some(candidate) = store.candidates.iter_mut().find(|candidate| {
            candidate.dedupe_key == dedupe_key
                && candidate
                    .fingerprint
                    .as_deref()
                    .is_none_or(|fingerprint| fingerprint.starts_with("legacy:"))
        }) {
            candidate.updated_at = now.clone();
            candidate.eligibility = ImprovementEligibility::NeedsEvidence;
            let next_state =
                settled_capture_state(&candidate.state, ImprovementEligibility::NeedsEvidence);
            transition_candidate(candidate, next_state)?;
            candidate.sanitized_summary = sanitized_summary.clone();
            candidate.sanitized_details = command.details.as_deref().map(sanitize_text);
            candidate.evidence_digest = command.evidence_digest.as_deref().map(sanitize_text);
            candidate.local_evidence = sanitize_local_evidence(&command.local_evidence);
            Ok(CaptureResult {
                candidate: candidate.clone(),
                updated_existing: true,
            })
        } else {
            let candidate = ImprovementCandidate {
                schema_version: super::improvement_store::STORE_SCHEMA_VERSION,
                id: format!("impr-{}", Uuid::new_v4().simple()),
                created_at: now.clone(),
                updated_at: now.clone(),
                source: command.source.clone(),
                target_artifact: command.target_artifact.clone(),
                classification: command.classification.clone(),
                confidence: command.confidence.clone(),
                state: CandidateState::NeedsEvidence,
                blocked_reason: None,
                failure_subcode: None,
                retry: None,
                owner: None,
                resolver_snapshot: None,
                dedupe_key: dedupe_key.clone(),
                occurrences: 0,
                legacy_occurrence_count: None,
                fingerprint: None,
                eligibility: ImprovementEligibility::NeedsEvidence,
                typed_evidence: None,
                distinct_occurrences: Vec::new(),
                capture_status_generation: u64::from(capture_claim_qualifies(command)),
                capture_status_delivered_generation: 0,
                owner_status_generation: 0,
                owner_status_delivered_generation: 0,
                reconciliation_required: false,
                reconciliation_owner_numbers: Vec::new(),
                sanitized_summary: sanitized_summary.clone(),
                sanitized_details: command.details.as_deref().map(sanitize_text),
                evidence_digest: command.evidence_digest.as_deref().map(sanitize_text),
                local_evidence: sanitize_local_evidence(&command.local_evidence),
                linked_issue: None,
                dismissed_reason: None,
                legacy_provenance: Vec::new(),
                pending_create_resolution: None,
                attempt: None,
                audit: Vec::new(),
            };
            store.candidates.push(candidate.clone());
            Ok(CaptureResult {
                candidate,
                updated_existing: false,
            })
        }
    })
}

fn validate_typed_evidence(
    input: &ImprovementTypedEvidenceCommand,
    target_artifact: &str,
) -> Result<TypedFailureEvidence, SpecOpsError> {
    if input.contract_schema_revision == 0 {
        return Err(invalid(
            "contract_schema_revision must be greater than zero",
        ));
    }
    Ok(TypedFailureEvidence {
        subsystem: normalize_lower_token("subsystem", &input.subsystem)?,
        contract_id: normalize_lower_token("contract_id", &input.contract_id)?,
        contract_schema_revision: input.contract_schema_revision,
        failure_code: normalize_upper_token("failure_code", &input.failure_code)?,
        target_artifact: target_artifact.to_string(),
        expected_outcome: normalize_upper_token("expected_outcome", &input.expected_outcome)?,
        observed_outcome: normalize_upper_token("observed_outcome", &input.observed_outcome)?,
    })
}

fn normalize_lower_token(field: &str, input: &str) -> Result<String, SpecOpsError> {
    validate_machine_token(field, &input.trim().to_ascii_lowercase())
}

fn normalize_upper_token(field: &str, input: &str) -> Result<String, SpecOpsError> {
    validate_machine_token(field, &input.trim().to_ascii_uppercase())
}

fn validate_machine_token(field: &str, normalized: &str) -> Result<String, SpecOpsError> {
    let valid = !normalized.is_empty()
        && normalized.len() <= 128
        && normalized
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'));
    if !valid {
        return Err(invalid(&format!(
            "{field} must be a machine token of at most 128 characters"
        )));
    }
    Ok(normalized.to_string())
}

pub(super) fn improvement_fingerprint(evidence: &TypedFailureEvidence) -> String {
    let revision = evidence.contract_schema_revision.to_string();
    format!(
        "v2:{}",
        digest_fields(
            "gwt.improvement.fingerprint.v2",
            &[
                &evidence.subsystem,
                &evidence.contract_id,
                &revision,
                &evidence.failure_code,
                &evidence.target_artifact,
            ],
        )
    )
}

pub(super) fn typed_evidence_digest(evidence: &TypedFailureEvidence) -> String {
    let revision = evidence.contract_schema_revision.to_string();
    digest_fields(
        "gwt.improvement.evidence.v2",
        &[
            &evidence.subsystem,
            &evidence.contract_id,
            &revision,
            &evidence.failure_code,
            &evidence.target_artifact,
            &evidence.expected_outcome,
            &evidence.observed_outcome,
        ],
    )
}

pub(super) fn occurrence_evidence_digest(
    evidence: &TypedFailureEvidence,
    recurrence: Option<&TypedRecurrenceEvidence>,
) -> String {
    let core_digest = typed_evidence_digest(evidence);
    let Some(recurrence) = recurrence else {
        return core_digest;
    };
    let (version_presence, installed_version) = recurrence
        .installed_version
        .as_deref()
        .map_or(("absent", ""), |value| ("present", value));
    let (commit_presence, build_commit) = recurrence
        .build_commit
        .as_deref()
        .map_or(("absent", ""), |value| ("present", value));
    digest_fields(
        "gwt.improvement.occurrence-evidence.v1",
        &[
            &core_digest,
            version_presence,
            installed_version,
            commit_presence,
            build_commit,
            &recurrence.observed_at,
        ],
    )
}

pub(super) fn opaque_occurrence_key(
    nonce: &str,
    fingerprint: &str,
    producer: &str,
    replay_key: &str,
) -> String {
    format!(
        "occ:v1:{}",
        digest_fields(
            "gwt.improvement.occurrence.v1",
            &[fingerprint, nonce, producer, replay_key],
        )
    )
}

fn digest_fields(domain: &str, fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    for value in std::iter::once(domain).chain(fields.iter().copied()) {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn resolver_revision(
    corpus_generation: &str,
    owner_candidates: &[OwnerCandidate],
) -> Result<String, SpecOpsError> {
    let mut canonical = owner_candidates
        .iter()
        .map(|candidate| serde_json::to_string(candidate).map_err(serde_as_spec_error))
        .collect::<Result<Vec<_>, _>>()?;
    canonical.sort();
    let mut fields = Vec::with_capacity(canonical.len() + 1);
    fields.push(corpus_generation);
    fields.extend(canonical.iter().map(String::as_str));
    Ok(digest_fields(
        "gwt.improvement.resolver-revision.v2",
        &fields,
    ))
}

fn validate_owner_candidate(owner: &OwnerCandidate) -> Result<(), SpecOpsError> {
    validate_owner_identity(owner.number, &owner.title, &owner.url)
}

fn validate_owner_identity(number: u64, title: &str, url: &str) -> Result<(), SpecOpsError> {
    if number == 0 {
        return Err(invalid("owner number must be greater than zero"));
    }
    if title.trim().is_empty() {
        return Err(invalid("owner title must not be empty"));
    }
    let expected_url = format!("https://github.com/{UPSTREAM_REPOSITORY}/issues/{number}");
    if url != expected_url {
        return Err(invalid(
            "owner URL does not match the upstream issue number",
        ));
    }
    Ok(())
}

pub(super) fn validate_candidate_lifecycle(
    candidate: &ImprovementCandidate,
) -> Result<(), SpecOpsError> {
    if candidate.capture_status_delivered_generation > candidate.capture_status_generation {
        return Err(invalid(
            "capture status delivery generation exceeds its queued generation",
        ));
    }
    if candidate.owner_status_delivered_generation > candidate.owner_status_generation {
        return Err(invalid(
            "owner status delivery generation exceeds its queued generation",
        ));
    }
    if !candidate.reconciliation_required && !candidate.reconciliation_owner_numbers.is_empty() {
        return Err(invalid(
            "reconciliation owner numbers require the reconciliation latch",
        ));
    }
    if candidate.reconciliation_owner_numbers.contains(&0)
        || candidate
            .reconciliation_owner_numbers
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid(
            "reconciliation owner numbers must be sorted unique positive values",
        ));
    }
    if candidate.pending_create_resolution.as_ref().is_some_and(|intent| {
        !matches!(
            intent,
            super::improvement_store::ResolutionAttemptIntent::CreateIssue { .. }
                | super::improvement_store::ResolutionAttemptIntent::CreateRegressionIssue { .. }
        )
    }) {
        return Err(invalid(
            "pending create resolution must contain a create intent",
        ));
    }
    match candidate.state {
        CandidateState::Blocked => {
            let reason = candidate
                .blocked_reason
                .ok_or_else(|| invalid("blocked candidate requires blocked_reason"))?;
            if candidate.retry.is_none() {
                return Err(invalid("blocked candidate requires retry metadata"));
            }
            if candidate.failure_subcode.is_some() && reason != BlockedReason::Search {
                return Err(invalid(
                    "failure_subcode is only valid for search blocked reason",
                ));
            }
        }
        CandidateState::RemoteOutcomeUnknown => {
            if candidate.retry.is_none() {
                return Err(invalid(
                    "remote-outcome-unknown candidate requires retry metadata",
                ));
            }
            if candidate.blocked_reason.is_some() || candidate.failure_subcode.is_some() {
                return Err(invalid(
                    "remote-outcome-unknown candidate cannot have blocked metadata",
                ));
            }
        }
        _ => {
            if candidate.blocked_reason.is_some() || candidate.failure_subcode.is_some() {
                return Err(invalid(
                    "blocked metadata is only valid for blocked candidates",
                ));
            }
            if candidate.retry.is_some() {
                return Err(invalid(
                    "retry metadata is only valid for blocked or remote outcomes",
                ));
            }
        }
    }

    if let Some(retry) = &candidate.retry {
        let normalized = normalize_upper_token("remediation", &retry.remediation)?;
        if normalized != retry.remediation {
            return Err(invalid("remediation must be an uppercase machine token"));
        }
        chrono::DateTime::parse_from_rfc3339(&retry.failed_at)
            .map_err(|_| invalid("retry failed_at must be RFC3339"))?;
    }

    if let Some(owner) = &candidate.owner {
        validate_owner_identity(owner.number, &owner.title, &owner.url)?;
        if owner.fingerprint.trim().is_empty()
            || candidate.fingerprint.as_deref() != Some(owner.fingerprint.as_str())
        {
            return Err(invalid("owner fingerprint does not match the candidate"));
        }
        chrono::DateTime::parse_from_rfc3339(&owner.readback_verified_at)
            .map_err(|_| invalid("owner readback_verified_at must be RFC3339"))?;
    }

    if let Some(snapshot) = &candidate.resolver_snapshot {
        if snapshot.corpus_generation.trim().is_empty() {
            return Err(invalid("resolver corpus generation must not be empty"));
        }
        for owner in &snapshot.owner_candidates {
            validate_owner_candidate(owner)?;
        }
        if snapshot.resolver_revision
            != resolver_revision(&snapshot.corpus_generation, &snapshot.owner_candidates)?
        {
            return Err(invalid("resolver revision does not match its snapshot"));
        }
    }

    for entry in &candidate.audit {
        match entry {
            ImprovementAuditEntry::ManualOwnerSelection {
                owner_number,
                resolver_revision,
                corpus_generation,
                recorded_at,
            } => {
                if *owner_number == 0
                    || resolver_revision.len() != 64
                    || !resolver_revision
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                    || corpus_generation.trim().is_empty()
                {
                    return Err(invalid("manual owner selection audit is invalid"));
                }
                chrono::DateTime::parse_from_rfc3339(recorded_at).map_err(|_| {
                    invalid("manual owner selection audit timestamp must be RFC3339")
                })?;
            }
        }
    }

    if matches!(
        candidate.state,
        CandidateState::Linked | CandidateState::Created
    ) && candidate.owner.is_none()
        && candidate.linked_issue.is_none()
    {
        return Err(invalid(
            "linked or created candidate requires a durable or legacy owner",
        ));
    }
    Ok(())
}

pub(super) fn candidate_transition_allowed(from: CandidateState, to: CandidateState) -> bool {
    if from == to {
        return true;
    }
    matches!(
        (from, to),
        (CandidateState::Pending, CandidateState::NeedsEvidence)
            | (CandidateState::Pending, CandidateState::OwnerResolving)
            | (
                CandidateState::NeedsEvidence,
                CandidateState::OwnerResolving
            )
            | (CandidateState::NeedsEvidence, CandidateState::Dismissed)
            | (CandidateState::OwnerResolving, CandidateState::Linked)
            | (CandidateState::OwnerResolving, CandidateState::Created)
            | (CandidateState::OwnerResolving, CandidateState::Blocked)
            | (
                CandidateState::OwnerResolving,
                CandidateState::RemoteOutcomeUnknown
            )
            | (CandidateState::OwnerResolving, CandidateState::Dismissed)
            | (CandidateState::Blocked, CandidateState::OwnerResolving)
            | (CandidateState::Blocked, CandidateState::Dismissed)
            | (
                CandidateState::RemoteOutcomeUnknown,
                CandidateState::OwnerResolving
            )
            | (CandidateState::RemoteOutcomeUnknown, CandidateState::Linked)
            | (
                CandidateState::RemoteOutcomeUnknown,
                CandidateState::Created
            )
            | (
                CandidateState::RemoteOutcomeUnknown,
                CandidateState::Dismissed
            )
            | (CandidateState::Linked, CandidateState::Recurrent)
            | (CandidateState::Linked, CandidateState::Dismissed)
            | (CandidateState::Created, CandidateState::Recurrent)
            | (CandidateState::Created, CandidateState::Dismissed)
            | (CandidateState::Recurrent, CandidateState::OwnerResolving)
            | (CandidateState::Recurrent, CandidateState::Blocked)
            | (CandidateState::Recurrent, CandidateState::Dismissed)
            | (CandidateState::Parked, CandidateState::NeedsEvidence)
            | (CandidateState::Parked, CandidateState::OwnerResolving)
            | (CandidateState::Parked, CandidateState::Dismissed)
    )
}

pub(super) fn transition_candidate(
    candidate: &mut ImprovementCandidate,
    next_state: CandidateState,
) -> Result<(), SpecOpsError> {
    if !candidate_transition_allowed(candidate.state, next_state) {
        return Err(invalid(&format!(
            "invalid improvement transition: {} -> {}",
            candidate.state.as_str(),
            next_state.as_str()
        )));
    }
    let mut next = candidate.clone();
    next.state = next_state;
    if !matches!(
        next_state,
        CandidateState::Blocked | CandidateState::RemoteOutcomeUnknown
    ) {
        next.blocked_reason = None;
        next.failure_subcode = None;
        next.retry = None;
    }
    validate_candidate_lifecycle(&next)?;
    *candidate = next;
    Ok(())
}

fn transition_to_owner_resolving(candidate: &mut ImprovementCandidate) -> Result<(), SpecOpsError> {
    if candidate.state == CandidateState::Pending {
        transition_candidate(candidate, CandidateState::NeedsEvidence)?;
    }
    transition_candidate(candidate, CandidateState::OwnerResolving)
}

fn typed_eligibility(candidate: &ImprovementCandidate) -> ImprovementEligibility {
    if candidate.distinct_occurrences.iter().any(|occurrence| {
        occurrence.origin == OccurrenceOrigin::Deterministic && occurrence.qualifies_unattended
    }) {
        return ImprovementEligibility::Deterministic;
    }
    if candidate
        .distinct_occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.origin == OccurrenceOrigin::Interpretive && occurrence.qualifies_unattended
        })
        .count()
        >= 2
    {
        ImprovementEligibility::InterpretiveCorroboration
    } else if capture_fields_qualify(
        &candidate.classification,
        &candidate.confidence,
        &candidate.target_artifact,
    ) {
        ImprovementEligibility::NeedsEvidence
    } else {
        ImprovementEligibility::Ineligible
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn owner_eligibility_is_canonical(
    candidate: &ImprovementCandidate,
    repo_root: &Path,
    canonical_source_scope_nonce: &str,
) -> bool {
    let Some(evidence) = candidate.typed_evidence.as_ref() else {
        return false;
    };
    if evidence.target_artifact != candidate.target_artifact
        || !capture_fields_qualify(
            &candidate.classification,
            &candidate.confidence,
            &candidate.target_artifact,
        )
        || candidate.occurrences != candidate.distinct_occurrences.len() as u64
    {
        return false;
    }

    let mut opaque_keys = BTreeSet::new();
    let mut source_scope_nonces = BTreeSet::new();
    let mut qualifying_deterministic = 0_usize;
    let mut qualifying_interpretive = 0_usize;
    let mut current_evidence_is_bound = false;
    let canonical_fingerprint = improvement_fingerprint(evidence);
    for occurrence in &candidate.distinct_occurrences {
        let opaque_digest = occurrence
            .opaque_key
            .strip_prefix("occ:v1:")
            .filter(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            });
        let occurrence_digest_is_canonical = occurrence.evidence_digest.len() == 64
            && occurrence
                .evidence_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'));
        current_evidence_is_bound |= occurrence.evidence_digest
            == occurrence_evidence_digest(evidence, occurrence.recurrence.as_ref());
        if opaque_digest.is_none()
            || !opaque_keys.insert(occurrence.opaque_key.as_str())
            || !occurrence_digest_is_canonical
        {
            return false;
        }

        let (source_scope_nonce, producer, replay_key) =
            match (occurrence.origin, occurrence.replay_proof.as_ref()) {
                (
                    OccurrenceOrigin::Deterministic,
                    Some(OccurrenceReplayProof::RegisteredEvent {
                        source_scope_nonce,
                        source_event_id,
                    }),
                ) if normalize_lower_token("source_event_id", source_event_id)
                    .is_ok_and(|normalized| normalized == *source_event_id) =>
                {
                    let Some(producer_id) = occurrence.producer_id.as_deref() else {
                        return false;
                    };
                    (source_scope_nonce, producer_id, source_event_id)
                }
                (
                    OccurrenceOrigin::Interpretive,
                    Some(OccurrenceReplayProof::InterpretiveSession {
                        source_scope_nonce,
                        session_id,
                    }),
                ) if interpretive_session_is_verified_for_repo(repo_root, session_id) => {
                    (source_scope_nonce, "json.interpretive", session_id)
                }
                _ => return false,
            };
        if source_scope_nonce != canonical_source_scope_nonce
            || !source_scope_nonce_is_canonical(source_scope_nonce)
            || occurrence.opaque_key
                != opaque_occurrence_key(
                    source_scope_nonce,
                    &canonical_fingerprint,
                    producer,
                    replay_key,
                )
        {
            return false;
        }
        source_scope_nonces.insert(source_scope_nonce.as_str());

        match occurrence.origin {
            OccurrenceOrigin::Deterministic => {
                let registered = REGISTERED_PRODUCERS.iter().any(|registration| {
                    occurrence.producer_id.as_deref() == Some(registration.producer_id)
                        && occurrence.producer_registry_revision == Some(PRODUCER_REGISTRY_REVISION)
                        && occurrence.routing_basis_revision
                            == Some(registration.routing_basis_revision)
                        && evidence.subsystem == registration.subsystem
                        && evidence.contract_id == registration.contract_id
                        && evidence.contract_schema_revision
                            == registration.contract_schema_revision
                        && evidence.target_artifact == registration.target_artifact
                        && (occurrence.recurrence.is_none() || registration.recurrence_capable)
                });
                if !registered {
                    return false;
                }
                qualifying_deterministic += usize::from(occurrence.qualifies_unattended);
            }
            OccurrenceOrigin::Interpretive => {
                if occurrence.producer_id.is_some()
                    || occurrence.producer_registry_revision.is_some()
                    || occurrence.routing_basis_revision.is_some()
                    || occurrence.recurrence.is_some()
                {
                    return false;
                }
                qualifying_interpretive += usize::from(occurrence.qualifies_unattended);
            }
        }
    }

    if source_scope_nonces.len() != 1 || !current_evidence_is_bound {
        return false;
    }

    match candidate.eligibility {
        ImprovementEligibility::Deterministic => qualifying_deterministic > 0,
        ImprovementEligibility::InterpretiveCorroboration => {
            qualifying_deterministic == 0 && qualifying_interpretive >= 2
        }
        ImprovementEligibility::NeedsEvidence | ImprovementEligibility::Ineligible => false,
    }
}

fn source_scope_nonce_is_canonical(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn capture_claim_qualifies(command: &ImprovementCaptureCommand) -> bool {
    capture_fields_qualify(
        &command.classification,
        &command.confidence,
        &command.target_artifact,
    )
}

fn capture_fields_qualify(classification: &str, confidence: &str, target_artifact: &str) -> bool {
    classification == "gwt-caused" && confidence == "high" && is_contract_artifact(target_artifact)
}

fn same_occurrence_replay(existing: &DistinctOccurrence, incoming: &DistinctOccurrence) -> bool {
    existing.evidence_digest == incoming.evidence_digest
        && existing.origin == incoming.origin
        && existing.qualifies_unattended == incoming.qualifies_unattended
        && existing.producer_id == incoming.producer_id
        && existing.producer_registry_revision == incoming.producer_registry_revision
        && existing.routing_basis_revision == incoming.routing_basis_revision
        && existing.replay_proof == incoming.replay_proof
        && existing.recurrence == incoming.recurrence
}

fn settled_capture_state(
    current: &CandidateState,
    eligibility: ImprovementEligibility,
) -> CandidateState {
    if !matches!(
        current,
        CandidateState::Pending | CandidateState::NeedsEvidence | CandidateState::Parked
    ) {
        return *current;
    }
    match eligibility {
        ImprovementEligibility::Deterministic
        | ImprovementEligibility::InterpretiveCorroboration => CandidateState::OwnerResolving,
        ImprovementEligibility::NeedsEvidence | ImprovementEligibility::Ineligible => {
            CandidateState::NeedsEvidence
        }
    }
}

fn settled_typed_capture_state(
    current: &CandidateState,
    eligibility: ImprovementEligibility,
) -> CandidateState {
    if matches!(current, CandidateState::Linked | CandidateState::Created) {
        CandidateState::Recurrent
    } else {
        settled_capture_state(current, eligibility)
    }
}

fn verified_interpretive_session(repo_root: &Path) -> Option<String> {
    let raw = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).ok()?;
    let parsed = Uuid::parse_str(raw.trim()).ok()?;
    let session_id = parsed.to_string();
    if raw.trim() != session_id {
        return None;
    }
    interpretive_session_is_verified_for_repo(repo_root, &session_id).then_some(session_id)
}

fn interpretive_session_is_verified_for_repo(repo_root: &Path, session_id: &str) -> bool {
    let Ok(parsed) = Uuid::parse_str(session_id) else {
        return false;
    };
    if parsed.to_string() != session_id {
        return false;
    }
    let path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return false;
    }
    let Ok(session) = gwt_agent::Session::load(&path) else {
        return false;
    };
    let expected_repo_hash = gwt_core::paths::project_scope_hash(repo_root).to_string();
    if session.id != session_id {
        return false;
    }
    let scope_matches = match session.repo_hash.as_deref() {
        Some(repo_hash) => repo_hash == expected_repo_hash,
        None => fs::symlink_metadata(&session.worktree_path)
            .ok()
            .is_some_and(|metadata| {
                !metadata.file_type().is_symlink()
                    && metadata.is_dir()
                    && gwt_core::paths::project_scope_hash(&session.worktree_path).as_str()
                        == expected_repo_hash
            }),
    };
    scope_matches
}

fn list(
    repo_root: &Path,
    command: ImprovementListCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let privacy_context = PublicMutationContext::for_repo(repo_root);
    let mut candidates = load_store(repo_root)?.candidates;
    candidates.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut values = Vec::new();
    for candidate in candidates {
        if let Some(state) = command.state {
            if candidate.state != state {
                continue;
            }
        }
        if let Some(reason) = command.blocked_reason {
            if candidate.blocked_reason != Some(reason) {
                continue;
            }
        }
        if let Some(subcode) = command.failure_subcode {
            if candidate.failure_subcode != Some(subcode) {
                continue;
            }
        }
        if let Some(classification) = &command.classification {
            if &candidate.classification != classification {
                continue;
            }
        }
        if let Some(confidence) = &command.confidence {
            if &candidate.confidence != confidence {
                continue;
            }
        }
        if let Some(owner_number) = command.owner_number {
            let durable_number = candidate.owner.as_ref().map(|owner| owner.number);
            let legacy_number = candidate.linked_issue.as_ref().map(|owner| owner.number);
            if durable_number != Some(owner_number) && legacy_number != Some(owner_number) {
                continue;
            }
        }
        values.push(candidate_public_json(&candidate, &privacy_context));
        if let Some(limit) = command.limit {
            if values.len() >= limit {
                break;
            }
        }
    }
    write_json(
        out,
        json!({
            "improvement_contract_version": 2,
            "candidates": values,
        }),
    )?;
    Ok(0)
}

fn dismiss(
    repo_root: &Path,
    command: ImprovementDismissCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if command.reason.trim().is_empty() {
        return Err(invalid("dismiss reason must not be empty"));
    }
    let privacy_context = PublicMutationContext::for_repo(repo_root);
    let now = Utc::now().to_rfc3339();
    let response = super::improvement_store::update(repo_root, |store| {
        let candidate = find_candidate_mut(store, &command.id)?;
        if candidate.state == CandidateState::Pending {
            transition_candidate(candidate, CandidateState::NeedsEvidence)?;
        }
        candidate.dismissed_reason = Some(sanitize_text(&command.reason));
        transition_candidate(candidate, CandidateState::Dismissed)?;
        candidate.updated_at = now;
        Ok(candidate_public_json(candidate, &privacy_context))
    })?;
    write_json(out, response)?;
    Ok(0)
}

fn link_issue(
    repo_root: &Path,
    command: ImprovementLinkIssueCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if command.number == 0 {
        return Err(invalid("issue number must be greater than zero"));
    }
    let repository = command
        .repository
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| UPSTREAM_REPOSITORY.to_string());
    let url = command
        .url
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "https://github.com/{repository}/issues/{number}",
                number = command.number
            )
        });
    let privacy_context = PublicMutationContext::for_repo(repo_root);
    let now = Utc::now().to_rfc3339();
    let response = super::improvement_store::update(repo_root, |store| {
        let candidate = find_candidate_mut(store, &command.id)?;
        if matches!(
            candidate.state,
            CandidateState::Linked | CandidateState::Created
        ) && candidate
            .linked_issue
            .as_ref()
            .is_some_and(|owner| owner.number == command.number)
        {
            return Ok(candidate_public_json(candidate, &privacy_context));
        }
        transition_to_owner_resolving(candidate)?;
        candidate.updated_at = now;
        candidate.linked_issue = Some(LinkedIssue {
            number: command.number,
            url: sanitize_text(&url),
            repository: sanitize_text(&repository),
        });
        transition_candidate(candidate, CandidateState::Linked)?;
        Ok(candidate_public_json(candidate, &privacy_context))
    })?;
    write_json(out, response)?;
    Ok(0)
}

fn promote_issue<E: CliEnv>(
    env: &mut E,
    command: ImprovementPromoteIssueCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if command.force {
        return Err(invalid("UNSAFE_FORCE_REMOVED"));
    }
    if !command.labels.is_empty() {
        return Err(invalid(
            "manual labels are not supported by Owner Resolution",
        ));
    }
    let repo_root = env.repo_path().to_path_buf();
    let store = load_store(&repo_root)?;
    let Some(index) = store
        .candidates
        .iter()
        .position(|candidate| candidate.id == command.id)
    else {
        return Err(invalid("candidate not found"));
    };
    let candidate = store.candidates[index].clone();
    if candidate.state == CandidateState::Dismissed {
        return Err(invalid("dismissed candidates cannot be promoted"));
    }
    if let Some(linked) = &candidate.linked_issue {
        if matches!(
            candidate.state,
            CandidateState::Created | CandidateState::Linked | CandidateState::Recurrent
        ) {
            write_json(
                out,
                json!({
                    "id": candidate.id,
                    "state": candidate.state.compatibility_state(),
                    "issue_number": linked.number,
                    "issue_url": linked.url,
                    "repository": linked.repository,
                    "already_linked": true,
                }),
            )?;
            return Ok(0);
        }
    }
    let candidate = resolve_candidate_owner(env, &command.id, CaptureBudgetProfile::Normal)?;
    let response = if let Some(linked) = &candidate.linked_issue {
        json!({
            "id": candidate.id,
            "state": candidate.state.compatibility_state(),
            "repository": linked.repository,
            "issue_number": linked.number,
            "issue_url": linked.url,
        })
    } else {
        json!({
            "id": candidate.id,
            "state": candidate.state.compatibility_state(),
            "blocked_reason": candidate.blocked_reason,
            "failure_subcode": candidate.failure_subcode,
        })
    };
    write_json(out, response)?;
    Ok(0)
}

fn queue_capture_status(candidate: &mut ImprovementCandidate) -> Result<(), SpecOpsError> {
    candidate.capture_status_generation = candidate
        .capture_status_generation
        .checked_add(1)
        .ok_or_else(|| invalid("capture status generation overflow"))?;
    Ok(())
}

fn pending_capture_status_generation(candidate: &ImprovementCandidate) -> Option<u64> {
    (candidate.capture_status_generation > candidate.capture_status_delivered_generation)
        .then_some(candidate.capture_status_generation)
}

fn should_resolve_after_capture(
    candidate: &ImprovementCandidate,
    pending_generation: Option<u64>,
) -> bool {
    candidate.state == CandidateState::OwnerResolving
        || (matches!(
            candidate.state,
            CandidateState::Blocked | CandidateState::Recurrent
        ) && pending_generation.is_some())
}

fn acknowledge_capture_status(
    repo_root: &Path,
    candidate_id: &str,
    generation: u64,
) -> Result<(), SpecOpsError> {
    super::improvement_store::update(repo_root, |store| {
        let candidate = find_candidate_mut(store, candidate_id)?;
        if generation == 0 || generation > candidate.capture_status_generation {
            return Err(invalid("capture status acknowledgement is invalid"));
        }
        candidate.capture_status_delivered_generation = candidate
            .capture_status_delivered_generation
            .max(generation);
        Ok(())
    })
}

fn post_candidate_captured_status<E: CliEnv>(
    env: &mut E,
    candidate: &ImprovementCandidate,
    updated_existing: bool,
) -> Result<(), SpecOpsError> {
    let status = if updated_existing {
        "updated"
    } else {
        "captured"
    };
    let body = format!(
        "Current state: Improvement Candidate {id} was {status} with high confidence for `{target}`.\n\nReason: {summary}\n\nNext: Owner Resolution runs automatically when the candidate is eligible; use Improvement Inbox only for audit or fail-closed remediation.",
        id = candidate.id,
        target = candidate.target_artifact,
        summary = candidate.sanitized_summary,
    );
    post_improvement_board_status(env, body)
}

pub(super) fn post_improvement_board_status<E: CliEnv>(
    env: &mut E,
    body: String,
) -> Result<(), SpecOpsError> {
    let mut board_out = String::new();
    super::board::run(
        env,
        super::BoardCommand::Post(Box::new(super::BoardPostCommand {
            kind: "status".to_string(),
            body: Some(body),
            file: None,
            title: Some("Improvement Candidate".to_string()),
            title_summary: Some("Improvement Candidate".to_string()),
            parent: None,
            topics: vec!["improvement".to_string()],
            owners: vec!["SPEC-3164".to_string()],
            targets: Vec::new(),
            mentions: Vec::new(),
            broadcast: true,
        })),
        &mut board_out,
    )?;
    Ok(())
}

fn find_candidate_mut<'a>(
    store: &'a mut CandidateStore,
    id: &str,
) -> Result<&'a mut ImprovementCandidate, SpecOpsError> {
    store
        .candidates
        .iter_mut()
        .find(|candidate| candidate.id == id)
        .ok_or_else(|| invalid("candidate not found"))
}

fn candidate_public_json(
    candidate: &ImprovementCandidate,
    privacy_context: &PublicMutationContext,
) -> Value {
    let resolver_revision = candidate
        .resolver_snapshot
        .as_ref()
        .map(|snapshot| snapshot.resolver_revision.as_str());
    let owner_candidates = candidate
        .resolver_snapshot
        .as_ref()
        .map(|snapshot| snapshot.owner_candidates.as_slice())
        .unwrap_or_default();
    let public_payload = render_public_issue_payload(candidate, privacy_context).ok();
    let issue_preview = public_payload.as_ref().map(|payload| {
        json!({
            "repository": UPSTREAM_REPOSITORY,
            "title": payload.title,
            "body": payload.body,
        })
    });
    json!({
        "id": candidate.id,
        "state": candidate.state.compatibility_state(),
        "resolution_state": candidate.state,
        "blocked_reason": candidate.blocked_reason,
        "failure_subcode": candidate.failure_subcode,
        "retry": candidate.retry,
        "owner": candidate.owner,
        "resolver_revision": resolver_revision,
        "owner_candidates": owner_candidates,
        "source": candidate.source,
        "target_artifact": candidate.target_artifact,
        "classification": candidate.classification,
        "confidence": candidate.confidence,
        "eligibility": candidate.eligibility,
        "occurrences": candidate.occurrences,
        "legacy_occurrence_count": candidate.legacy_occurrence_count,
        "fingerprint": candidate.fingerprint,
        "summary": public_payload
            .as_ref()
            .map(|payload| payload.summary.as_str())
            .unwrap_or("Public preview unavailable"),
        "linked_issue": candidate.linked_issue,
        "dismissed_reason": candidate.dismissed_reason.as_ref().map(|_| "Dismissed"),
        "updated_at": candidate.updated_at,
        "issue_preview": issue_preview,
    })
}

fn load_store(repo_root: &Path) -> Result<CandidateStore, SpecOpsError> {
    super::improvement_owner::repair_source_success_snapshots(repo_root)?;
    super::improvement_store::load_and_repair(repo_root)
}

fn load_store_with_projection_fallback(repo_root: &Path) -> Result<CandidateStore, SpecOpsError> {
    load_store(repo_root).or_else(|_| super::improvement_store::load_and_repair(repo_root))
}

fn sanitize_local_evidence(items: &[Value]) -> Vec<LocalEvidenceReference> {
    items
        .iter()
        .filter_map(|value| {
            let object = value.as_object()?;
            let kind = object
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("evidence");
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .map(sanitize_text);
            Some(LocalEvidenceReference {
                kind: sanitize_text(kind),
                path,
                digest: None,
            })
        })
        .collect()
}

fn sanitize_text(input: &str) -> String {
    let token_re =
        Regex::new(r"\b(?:gh[pousr]|github_pat)_[A-Za-z0-9_]+\b").expect("valid token regex");
    let path_re = Regex::new(r"(?:/[A-Za-z0-9._\-\s]+){2,}").expect("valid path regex");
    let mut out = token_re.replace_all(input, "[redacted-secret]").to_string();
    out = path_re.replace_all(&out, "[redacted-path]").to_string();
    if out.chars().count() > 2_000 {
        out = truncate_chars(&out, 2_000);
        out.push_str("...[truncated]");
    }
    out
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut out = input.chars().take(max_chars).collect::<String>();
    if input.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn default_dedupe_key(command: &ImprovementCaptureCommand, summary: &str) -> String {
    let slug = summary
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "candidate".to_string()
    } else {
        slug
    };
    format!(
        "{}:{}:{}",
        command.classification, command.target_artifact, slug
    )
}

fn validate_enum(flag: &'static str, value: &str, allowed: &[&str]) -> Result<(), SpecOpsError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(SpecOpsError::from(ApiError::Unexpected(format!(
            "invalid value for {flag}: {value}"
        ))))
    }
}

fn write_json(out: &mut String, value: Value) -> Result<(), SpecOpsError> {
    out.push_str(&serde_json::to_string(&value).map_err(serde_as_spec_error)?);
    out.push('\n');
    Ok(())
}

fn invalid(message: &str) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(message.to_string()))
}

fn serde_as_spec_error(err: serde_json::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(err.to_string()))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::cli::{run_collect, BoardCommand, CliCommand, TestEnv};

    fn parse_output(output: &str) -> Value {
        serde_json::from_str(output.trim()).expect("JSON output")
    }

    fn capture_command(command: ImprovementCaptureCommand) -> ImprovementCommand {
        ImprovementCommand::Capture(Box::new(command))
    }

    fn board_entries_json(env: &mut TestEnv) -> Value {
        let (_, board_out) = run_collect(
            env,
            CliCommand::Board(BoardCommand::Show {
                json: true,
                workspace: None,
                all: true,
            }),
        )
        .expect("board show");
        serde_json::from_str(&board_out).expect("board JSON")
    }

    fn board_bodies(env: &mut TestEnv) -> Vec<String> {
        board_entries_json(env)["board"]["entries"]
            .as_array()
            .expect("board entries")
            .iter()
            .map(|entry| {
                entry["body"]
                    .as_str()
                    .expect("board entry body")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn high_confidence_capture_posts_board_status() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let (capture_code, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(capture_command(ImprovementCaptureCommand {
                source: "agent-failure".to_string(),
                target_artifact: "skill".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "high".to_string(),
                summary: "Skill loop missed a required update".to_string(),
                details: None,
                evidence_digest: Some("Stop hook caught the missing skill update.".to_string()),
                dedupe_key: Some("skill:missed-required-update".to_string()),
                local_evidence: Vec::new(),
                typed_evidence: None,
            })),
        )
        .expect("capture");

        assert_eq!(capture_code, 0);
        let id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("candidate id")
            .to_string();
        let bodies = board_bodies(&mut env);
        assert_eq!(
            bodies.len(),
            1,
            "capture should post exactly one board entry"
        );
        assert!(
            bodies[0].contains(&id),
            "board entry should mention candidate id"
        );
        assert!(
            bodies[0].contains("Skill loop missed a required update"),
            "board entry should include sanitized summary: {}",
            bodies[0]
        );
        assert!(
            bodies[0].contains("Improvement Candidate"),
            "board entry should identify the improvement candidate: {}",
            bodies[0]
        );
    }

    #[test]
    fn low_confidence_capture_does_not_post_board_status() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        run_collect(
            &mut env,
            CliCommand::Improvement(capture_command(ImprovementCaptureCommand {
                source: "verification".to_string(),
                target_artifact: "verification".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "low".to_string(),
                summary: "Weak verification signal".to_string(),
                details: None,
                evidence_digest: None,
                dedupe_key: Some("verification:weak-signal-board".to_string()),
                local_evidence: Vec::new(),
                typed_evidence: None,
            })),
        )
        .expect("capture");

        let bodies = board_bodies(&mut env);
        assert!(
            bodies.is_empty(),
            "low-confidence capture should stay local without a board post: {bodies:?}"
        );
    }

    #[test]
    fn promote_issue_rejects_untyped_candidate_without_owner_mutation() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let (_, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(capture_command(ImprovementCaptureCommand {
                source: "agent-failure".to_string(),
                target_artifact: "coordination".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "high".to_string(),
                summary: "Board guidance drift".to_string(),
                details: None,
                evidence_digest: Some("Agent missed required Board update.".to_string()),
                dedupe_key: Some("coordination:board-guidance-drift-post".to_string()),
                local_evidence: Vec::new(),
                typed_evidence: None,
            })),
        )
        .expect("capture");
        let id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("candidate id")
            .to_string();

        let error = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: id.clone(),
                    force: false,
                    labels: Vec::new(),
                },
            )),
        )
        .expect_err("untyped candidate must not enter Owner Resolution");

        let bodies = board_bodies(&mut env);
        assert!(error.to_string().contains("not eligible"));
        assert_eq!(bodies.len(), 1, "rejected promote must not post status");
        assert!(bodies[0].contains(&id));
        assert!(env.owner_client.owner_call_log().is_empty());
        assert!(env.owner_client.owner_mutation_call_log().is_empty());
        assert!(env.target_issue_create_call_log.is_empty());
    }

    #[test]
    fn promote_issue_rejects_manual_labels_before_owner_transport() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let skill_path = env.repo_path.join(".codex/skills/gwt-discussion/SKILL.md");
        std::fs::create_dir_all(skill_path.parent().expect("skill parent")).expect("skill dir");
        std::fs::write(&skill_path, "original skill").expect("skill file");

        let (capture_code, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(capture_command(ImprovementCaptureCommand {
                source: "agent-failure".to_string(),
                target_artifact: "skill".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "high".to_string(),
                summary: "Skill update missing for /Users/alice/private-app".to_string(),
                details: Some(
                    "Failure came from /Users/alice/private-app/AGENTS.md and token ghp_1234567890abcdef."
                        .to_string(),
                ),
                evidence_digest: Some("Stop hook allowed completion without capture.".to_string()),
                dedupe_key: Some("skill:gwt-discussion:self-improvement".to_string()),
                local_evidence: Vec::new(),
                typed_evidence: None,
            })),
        )
        .expect("capture");
        assert_eq!(capture_code, 0);
        let id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("id")
            .to_string();
        let (_, list_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::List(ImprovementListCommand {
                state: Some(CandidateState::NeedsEvidence),
                blocked_reason: None,
                failure_subcode: None,
                classification: None,
                confidence: None,
                owner_number: None,
                limit: None,
            })),
        )
        .expect("preview before mutation");
        let listed = parse_output(&list_out);
        let preview_title = listed["candidates"][0]["issue_preview"]["title"]
            .as_str()
            .expect("preview title")
            .to_string();
        let preview_body = listed["candidates"][0]["issue_preview"]["body"]
            .as_str()
            .expect("preview body")
            .to_string();

        let error = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: id.clone(),
                    force: false,
                    labels: vec!["bug".to_string()],
                },
            )),
        )
        .expect_err("manual labels must be rejected before candidate lookup");

        assert!(error
            .to_string()
            .contains("manual labels are not supported"));
        assert!(!preview_title.contains("/Users/alice"));
        assert!(
            !preview_body.contains("/Users/alice"),
            "public preview must not contain private paths: {preview_body}"
        );
        assert!(
            !preview_body.contains("ghp_1234567890abcdef"),
            "public preview must not contain token-like secrets: {preview_body}"
        );
        assert!(env.owner_client.owner_call_log().is_empty());
        assert!(env.owner_client.owner_mutation_call_log().is_empty());
        assert!(env.target_issue_create_call_log.is_empty());
        assert_eq!(
            std::fs::read_to_string(&skill_path).expect("skill file"),
            "original skill",
            "rejected promotion must not mutate skill files"
        );
    }

    #[test]
    fn list_candidates_includes_sanitized_upstream_issue_preview() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let (capture_code, _capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(capture_command(ImprovementCaptureCommand {
                source: "agent-failure".to_string(),
                target_artifact: "skill".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "high".to_string(),
                summary: "Skill update missing for /Users/alice/private-app".to_string(),
                details: Some(
                    "Failure came from /Users/alice/private-app/AGENTS.md and token ghp_1234567890abcdef."
                        .to_string(),
                ),
                evidence_digest: Some("Stop hook allowed completion without capture.".to_string()),
                dedupe_key: Some("skill:gwt-discussion:self-improvement".to_string()),
                local_evidence: Vec::new(),
                typed_evidence: None,
            })),
        )
        .expect("capture");
        assert_eq!(capture_code, 0);

        let (list_code, list_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::List(ImprovementListCommand {
                state: Some(CandidateState::NeedsEvidence),
                blocked_reason: None,
                failure_subcode: None,
                classification: None,
                confidence: None,
                owner_number: None,
                limit: None,
            })),
        )
        .expect("list");

        assert_eq!(list_code, 0, "list output: {list_out}");
        let output = parse_output(&list_out);
        let preview = &output["candidates"][0]["issue_preview"];
        assert_eq!(preview["repository"], "akiojin/gwt");
        assert_eq!(
            preview["title"],
            "fix(gwt): Self-improvement contract evidence required"
        );
        let body = preview["body"].as_str().expect("preview body");
        assert!(body.contains("## Problem"));
        assert!(body.contains("## Expected behavior"));
        assert!(body.contains("## Observed evidence"));
        assert!(body.contains("## Impact"));
        assert!(body.contains("## Suggested verification"));
        assert!(body.contains("## Source candidate"));
        assert!(body.contains("## Privacy"));
        assert!(body.contains("Typed evidence is required before unattended owner resolution."));
        assert!(!body.contains("Stop hook allowed completion without capture."));
        assert!(!body.contains("Skill update missing"));
        assert!(body.contains("- Target artifact: skill"));
        assert!(
            !body.contains("/Users/alice"),
            "preview body must not contain private paths: {body}"
        );
        assert!(
            !body.contains("ghp_1234567890abcdef"),
            "preview body must not contain token-like secrets: {body}"
        );
    }

    #[test]
    fn promote_issue_requires_owner_eligible_candidate() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let (_, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(capture_command(ImprovementCaptureCommand {
                source: "verification".to_string(),
                target_artifact: "verification".to_string(),
                classification: "gwt-caused".to_string(),
                confidence: "low".to_string(),
                summary: "Weak signal should stay local".to_string(),
                details: None,
                evidence_digest: None,
                dedupe_key: Some("verification:weak-signal".to_string()),
                local_evidence: Vec::new(),
                typed_evidence: None,
            })),
        )
        .expect("capture");
        let low_id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("low id")
            .to_string();

        let err = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: low_id,
                    force: false,
                    labels: Vec::new(),
                },
            )),
        )
        .expect_err("low-confidence promotion should be rejected");
        assert!(
            err.to_string().contains("not eligible"),
            "unexpected error: {err}"
        );
        assert!(env.owner_client.owner_mutation_call_log().is_empty());
        assert!(env.target_issue_create_call_log.is_empty());

        let (_, capture_out) = run_collect(
            &mut env,
            CliCommand::Improvement(capture_command(ImprovementCaptureCommand {
                source: "manual".to_string(),
                target_artifact: "unknown".to_string(),
                classification: "target-project".to_string(),
                confidence: "high".to_string(),
                summary: "Target project failure should not auto-promote".to_string(),
                details: None,
                evidence_digest: None,
                dedupe_key: Some("target-project:not-gwt".to_string()),
                local_evidence: Vec::new(),
                typed_evidence: None,
            })),
        )
        .expect("capture");
        let target_id = parse_output(&capture_out)["id"]
            .as_str()
            .expect("target id")
            .to_string();
        let err = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: target_id,
                    force: false,
                    labels: Vec::new(),
                },
            )),
        )
        .expect_err("non-gwt-caused promotion should be rejected");
        assert!(
            err.to_string().contains("not eligible"),
            "unexpected error: {err}"
        );
        assert!(env.owner_client.owner_mutation_call_log().is_empty());
        assert!(env.target_issue_create_call_log.is_empty());
    }

    #[test]
    fn resolved_capture_preserves_linked_owner_and_promote_is_idempotent() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        seed_revision_pinned_active_owner(&mut env);
        let token = registered_producer_token("test.owner-route").expect("registered token");

        let first = capture_registered(
            &mut env,
            registered_capture_input(token, "idempotent", 9, CaptureBudgetProfile::Normal),
        )
        .expect("resolved capture");
        assert_eq!(first.state, CandidateState::Linked);
        assert_eq!(first.linked_issue.as_ref().unwrap().number, 77);
        let mutation_count = env.owner_client.owner_mutation_count();

        let replay = capture_registered(
            &mut env,
            registered_capture_input(token, "idempotent", 9, CaptureBudgetProfile::Normal),
        )
        .expect("replayed capture");
        assert_eq!(replay.id, first.id);
        assert_eq!(replay.state, CandidateState::Linked);
        assert_eq!(env.owner_client.owner_mutation_count(), mutation_count);

        let (_, promote_out) = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: first.id,
                    force: false,
                    labels: Vec::new(),
                },
            )),
        )
        .expect("idempotent promote");
        let promoted = parse_output(&promote_out);
        assert_eq!(promoted["already_linked"], true);
        assert_eq!(
            env.owner_client.owner_mutation_count(),
            mutation_count,
            "already linked candidate must not repeat owner mutation"
        );
    }

    #[test]
    fn registered_recapture_updates_the_active_owner_occurrence() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("target-project");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        seed_revision_pinned_active_owner(&mut env);
        let token = registered_producer_token("test.owner-route").expect("registered token");

        let first = capture_registered(
            &mut env,
            registered_capture_input(
                token,
                "active-occurrence-a",
                9,
                CaptureBudgetProfile::Normal,
            ),
        )
        .expect("first linked occurrence");
        assert_eq!(first.state, CandidateState::Linked);
        let first_mutations = env.owner_client.owner_mutation_count();

        let recurrent = capture_registered(
            &mut env,
            registered_capture_input(
                token,
                "active-occurrence-b",
                9,
                CaptureBudgetProfile::Normal,
            ),
        )
        .expect("recurrent occurrence resolution");

        assert_eq!(recurrent.state, CandidateState::Linked);
        assert_eq!(recurrent.occurrences, 2);
        assert_eq!(recurrent.owner.as_ref().unwrap().number, 77);
        assert_eq!(
            env.owner_client.owner_mutation_count(),
            first_mutations + 1,
            "only the new immutable occurrence comment should be added"
        );
    }

    fn registered_capture_input(
        token: RegisteredProducerToken,
        source_event_id: &str,
        routing_basis_revision: u64,
        budget_profile: CaptureBudgetProfile,
    ) -> RegisteredCaptureInput {
        RegisteredCaptureInput {
            token,
            source_event_id: source_event_id.to_string(),
            routing_basis_revision,
            budget_profile,
            source: "hook-runtime".to_string(),
            target_artifact: "coordination".to_string(),
            classification: "gwt-caused".to_string(),
            confidence: "high".to_string(),
            failure_code: "STATUS_NOT_POSTED".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            observed_outcome: "BOARD_STATUS_MISSING".to_string(),
            summary: Some("Local deterministic capture context".to_string()),
            details: None,
            local_evidence: Vec::new(),
            recurrence: None,
        }
    }

    fn capture_registered_candidate_without_resolution(
        env: &mut TestEnv,
        source_event_id: &str,
    ) -> ImprovementCandidate {
        let evidence = TypedFailureEvidence {
            subsystem: "coordination".to_string(),
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            target_artifact: "coordination".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            observed_outcome: "BOARD_STATUS_MISSING".to_string(),
        };
        let command = ImprovementCaptureCommand {
            source: "hook-runtime".to_string(),
            target_artifact: "coordination".to_string(),
            classification: "gwt-caused".to_string(),
            confidence: "high".to_string(),
            summary: "Local deterministic capture context".to_string(),
            details: None,
            evidence_digest: None,
            dedupe_key: None,
            local_evidence: Vec::new(),
            typed_evidence: None,
        };
        let candidate = capture_typed(
            &env.repo_path,
            &command,
            evidence,
            ValidatedCaptureOrigin::Registered {
                producer_id: "test.coordination-gate.v1",
                source_event_id: source_event_id.to_string(),
                producer_registry_revision: PRODUCER_REGISTRY_REVISION,
                routing_basis_revision: 1,
                recurrence: None,
            },
        )
        .expect("capture candidate")
        .candidate;
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        candidate
    }

    fn seed_revision_pinned_active_owner(env: &mut TestEnv) {
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        let fingerprint = improvement_fingerprint(&TypedFailureEvidence {
            subsystem: "coordination".to_string(),
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            target_artifact: "coordination".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            observed_outcome: "BOARD_STATUS_MISSING".to_string(),
        });
        env.owner_client
            .seed_repository_issue(gwt_github::client::RepositoryIssue {
                repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                number: gwt_github::client::IssueNumber(77),
                title: "Revision-pinned active owner".to_string(),
                body: format!("<!-- gwt:improvement-fingerprint:v1 {fingerprint} -->"),
                labels: Vec::new(),
                state: gwt_github::client::IssueState::Open,
                kind: gwt_github::client::RepositoryIssueKind::Plain,
                updated_at: gwt_github::client::UpdatedAt::new("u77"),
            });
    }

    fn seed_exact_plain_owners(env: &mut TestEnv, numbers: &[u64]) {
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        let fingerprint = improvement_fingerprint(&TypedFailureEvidence {
            subsystem: "coordination".to_string(),
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            target_artifact: "coordination".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            observed_outcome: "BOARD_STATUS_MISSING".to_string(),
        });
        for number in numbers {
            env.owner_client
                .seed_repository_issue(gwt_github::client::RepositoryIssue {
                    repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                    number: gwt_github::client::IssueNumber(*number),
                    title: format!("Duplicate owner {number}"),
                    body: format!("<!-- gwt:improvement-fingerprint:v1 {fingerprint} -->"),
                    labels: Vec::new(),
                    state: gwt_github::client::IssueState::Open,
                    kind: gwt_github::client::RepositoryIssueKind::Plain,
                    updated_at: gwt_github::client::UpdatedAt::new(format!("u{number}")),
                });
        }
    }

    fn lifecycle_test_candidate(state: CandidateState) -> ImprovementCandidate {
        serde_json::from_value(json!({
            "schema_version": 3,
            "id": "impr-lifecycle",
            "created_at": "2026-07-14T00:00:00Z",
            "updated_at": "2026-07-14T00:00:00Z",
            "source": "agent-failure",
            "target_artifact": "coordination",
            "classification": "gwt-caused",
            "confidence": "high",
            "state": state,
            "dedupe_key": "lifecycle:test",
            "occurrences": 1,
            "fingerprint": "v2:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "eligibility": "deterministic",
            "sanitized_summary": "Lifecycle test",
            "sanitized_details": null,
            "evidence_digest": null,
            "local_evidence": [],
            "linked_issue": null,
            "dismissed_reason": null
        }))
        .expect("lifecycle candidate")
    }

    fn retry_metadata() -> RetryMetadata {
        RetryMetadata {
            retryable: true,
            remediation: "REFRESH_OWNER_CORPUS".to_string(),
            failed_at: "2026-07-14T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn candidate_state_serde_and_transition_matrix_is_exact() {
        let states = [
            (CandidateState::Pending, "pending"),
            (CandidateState::NeedsEvidence, "needs-evidence"),
            (CandidateState::OwnerResolving, "owner-resolving"),
            (CandidateState::Linked, "linked"),
            (CandidateState::Created, "created"),
            (CandidateState::Blocked, "blocked"),
            (
                CandidateState::RemoteOutcomeUnknown,
                "remote-outcome-unknown",
            ),
            (CandidateState::Recurrent, "recurrent"),
            (CandidateState::Parked, "parked"),
            (CandidateState::Dismissed, "dismissed"),
        ];
        for (state, serialized) in states {
            assert_eq!(serde_json::to_value(state).unwrap(), serialized);
            assert_eq!(
                serde_json::from_value::<CandidateState>(json!(serialized)).unwrap(),
                state
            );
        }
        assert_eq!(
            serde_json::from_value::<CandidateState>(json!("promoted")).unwrap(),
            CandidateState::Created
        );
        for forbidden in ["queue", "in-progress", "verification-pending", "resolved"] {
            assert!(serde_json::from_value::<CandidateState>(json!(forbidden)).is_err());
        }

        let allowed = [
            (CandidateState::Pending, CandidateState::NeedsEvidence),
            (CandidateState::Pending, CandidateState::OwnerResolving),
            (
                CandidateState::NeedsEvidence,
                CandidateState::OwnerResolving,
            ),
            (CandidateState::NeedsEvidence, CandidateState::Dismissed),
            (CandidateState::OwnerResolving, CandidateState::Linked),
            (CandidateState::OwnerResolving, CandidateState::Created),
            (CandidateState::OwnerResolving, CandidateState::Blocked),
            (
                CandidateState::OwnerResolving,
                CandidateState::RemoteOutcomeUnknown,
            ),
            (CandidateState::OwnerResolving, CandidateState::Dismissed),
            (CandidateState::Blocked, CandidateState::OwnerResolving),
            (CandidateState::Blocked, CandidateState::Dismissed),
            (
                CandidateState::RemoteOutcomeUnknown,
                CandidateState::OwnerResolving,
            ),
            (CandidateState::RemoteOutcomeUnknown, CandidateState::Linked),
            (
                CandidateState::RemoteOutcomeUnknown,
                CandidateState::Created,
            ),
            (
                CandidateState::RemoteOutcomeUnknown,
                CandidateState::Dismissed,
            ),
            (CandidateState::Linked, CandidateState::Recurrent),
            (CandidateState::Linked, CandidateState::Dismissed),
            (CandidateState::Created, CandidateState::Recurrent),
            (CandidateState::Created, CandidateState::Dismissed),
            (CandidateState::Recurrent, CandidateState::OwnerResolving),
            (CandidateState::Recurrent, CandidateState::Blocked),
            (CandidateState::Recurrent, CandidateState::Dismissed),
            (CandidateState::Parked, CandidateState::NeedsEvidence),
            (CandidateState::Parked, CandidateState::OwnerResolving),
            (CandidateState::Parked, CandidateState::Dismissed),
        ];
        for (from, _) in states {
            for (to, _) in states {
                assert_eq!(
                    candidate_transition_allowed(from, to),
                    from == to || allowed.contains(&(from, to)),
                    "unexpected transition {from:?} -> {to:?}"
                );
            }
        }
    }

    #[test]
    fn candidate_lifecycle_rejects_non_create_pending_resolution_root() {
        let mut candidate = lifecycle_test_candidate(CandidateState::OwnerResolving);
        candidate.pending_create_resolution = Some(
            super::super::improvement_store::ResolutionAttemptIntent::ReconciliationComment {
                canonical_owner_number: 41,
                duplicate_owner_number: 42,
                public_payload_digest: "sha256:reconciliation".to_string(),
            },
        );

        assert!(validate_candidate_lifecycle(&candidate).is_err());

        candidate.pending_create_resolution = Some(
            super::super::improvement_store::ResolutionAttemptIntent::CreateIssue {
                fingerprint: "v2:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
                public_payload_digest: "sha256:create".to_string(),
                created_owner_number: None,
            },
        );
        validate_candidate_lifecycle(&candidate).expect("create root is lifecycle-valid");
    }

    #[test]
    fn typed_recapture_moves_owned_candidates_to_recurrent() {
        for state in [CandidateState::Linked, CandidateState::Created] {
            assert_eq!(
                settled_typed_capture_state(&state, ImprovementEligibility::Deterministic),
                CandidateState::Recurrent
            );
        }
        assert_eq!(
            settled_typed_capture_state(
                &CandidateState::NeedsEvidence,
                ImprovementEligibility::Deterministic,
            ),
            CandidateState::OwnerResolving
        );
    }

    #[test]
    fn blocked_reason_subcode_and_retry_matrix_is_validated() {
        let reasons = [
            BlockedReason::Store,
            BlockedReason::Search,
            BlockedReason::Auth,
            BlockedReason::Privacy,
            BlockedReason::Ambiguity,
            BlockedReason::Routing,
            BlockedReason::Create,
            BlockedReason::Update,
            BlockedReason::Readback,
            BlockedReason::LocalCommit,
            BlockedReason::Timeout,
            BlockedReason::RateLimit,
            BlockedReason::Network,
            BlockedReason::Parse,
            BlockedReason::Reconciliation,
        ];
        for reason in reasons {
            let mut candidate = lifecycle_test_candidate(CandidateState::Blocked);
            candidate.blocked_reason = Some(reason);
            candidate.retry = Some(retry_metadata());
            candidate.failure_subcode = None;
            validate_candidate_lifecycle(&candidate).expect("valid blocked candidate");

            for subcode in [FailureSubcode::EmptyCorpus, FailureSubcode::PartialPage] {
                candidate.failure_subcode = Some(subcode);
                if reason == BlockedReason::Search {
                    validate_candidate_lifecycle(&candidate)
                        .expect("search failure subcode must be valid");
                } else {
                    assert!(validate_candidate_lifecycle(&candidate).is_err());
                }
            }
        }

        let mut invalid = lifecycle_test_candidate(CandidateState::Blocked);
        invalid.retry = Some(retry_metadata());
        assert!(validate_candidate_lifecycle(&invalid).is_err());
        invalid.blocked_reason = Some(BlockedReason::Auth);
        invalid.failure_subcode = Some(FailureSubcode::PartialPage);
        assert!(validate_candidate_lifecycle(&invalid).is_err());

        let mut invalid = lifecycle_test_candidate(CandidateState::NeedsEvidence);
        invalid.blocked_reason = Some(BlockedReason::Search);
        assert!(validate_candidate_lifecycle(&invalid).is_err());

        let mut remote = lifecycle_test_candidate(CandidateState::RemoteOutcomeUnknown);
        assert!(validate_candidate_lifecycle(&remote).is_err());
        remote.retry = Some(retry_metadata());
        validate_candidate_lifecycle(&remote).expect("remote retry metadata");
        remote.retry.as_mut().unwrap().remediation = "retry later".to_string();
        assert!(validate_candidate_lifecycle(&remote).is_err());
        remote.retry.as_mut().unwrap().remediation = "REFRESH_OWNER_CORPUS".to_string();
        remote.retry.as_mut().unwrap().failed_at = "not-a-time".to_string();
        assert!(validate_candidate_lifecycle(&remote).is_err());
    }

    #[test]
    fn resolver_revision_is_order_independent_and_tamper_evident() {
        let owner = |number, title: &str| OwnerCandidate {
            number,
            kind: OwnerKind::Issue,
            title: title.to_string(),
            active: true,
            url: format!("https://github.com/akiojin/gwt/issues/{number}"),
            match_basis: OwnerMatchBasis::Fingerprint,
            selectable: true,
        };
        let first = ResolverSnapshot::new(
            "generation-a".to_string(),
            vec![owner(42, "Owner B"), owner(7, "Owner A")],
        )
        .expect("resolver snapshot");
        let reordered = ResolverSnapshot::new(
            "generation-a".to_string(),
            vec![owner(7, "Owner A"), owner(42, "Owner B")],
        )
        .expect("reordered snapshot");
        assert_eq!(first.resolver_revision, reordered.resolver_revision);
        assert_ne!(
            first.resolver_revision,
            ResolverSnapshot::new(
                "generation-b".to_string(),
                vec![owner(7, "Owner A"), owner(42, "Owner B")],
            )
            .unwrap()
            .resolver_revision
        );
        assert_ne!(
            first.resolver_revision,
            ResolverSnapshot::new(
                "generation-a".to_string(),
                vec![owner(7, "Changed title"), owner(42, "Owner B")],
            )
            .unwrap()
            .resolver_revision
        );

        let mut candidate = lifecycle_test_candidate(CandidateState::Blocked);
        candidate.blocked_reason = Some(BlockedReason::Ambiguity);
        candidate.retry = Some(retry_metadata());
        candidate.resolver_snapshot = Some(first.clone());
        validate_candidate_lifecycle(&candidate).expect("valid resolver revision");
        candidate
            .resolver_snapshot
            .as_mut()
            .unwrap()
            .resolver_revision = "0".repeat(64);
        assert!(validate_candidate_lifecycle(&candidate).is_err());
    }

    #[test]
    fn candidate_public_projection_contains_typed_owner_candidates_without_generation() {
        let mut candidate = lifecycle_test_candidate(CandidateState::Blocked);
        candidate.blocked_reason = Some(BlockedReason::Ambiguity);
        candidate.failure_subcode = None;
        candidate.retry = Some(retry_metadata());
        candidate.resolver_snapshot = Some(
            ResolverSnapshot::new(
                "private-generation".to_string(),
                vec![OwnerCandidate {
                    number: 42,
                    kind: OwnerKind::Spec,
                    title: "Public SPEC title".to_string(),
                    active: true,
                    url: "https://github.com/akiojin/gwt/issues/42".to_string(),
                    match_basis: OwnerMatchBasis::Contract,
                    selectable: true,
                }],
            )
            .unwrap(),
        );
        let value = candidate_public_json(&candidate, &PublicMutationContext::default());
        assert_eq!(value["resolution_state"], "blocked");
        assert_eq!(value["blocked_reason"], "ambiguity");
        assert_eq!(value["owner_candidates"][0]["kind"], "spec");
        assert_eq!(value["owner_candidates"][0]["match_basis"], "contract");
        assert!(value["resolver_revision"].as_str().is_some());
        assert!(value.get("corpus_generation").is_none());
    }

    #[test]
    fn typed_identity_has_stable_golden_vectors() {
        let evidence = TypedFailureEvidence {
            subsystem: "coordination".to_string(),
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            target_artifact: "coordination".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            observed_outcome: "BOARD_STATUS_MISSING".to_string(),
        };
        let fingerprint = improvement_fingerprint(&evidence);
        assert_eq!(
            fingerprint,
            "v2:4bea839977a5aeedbf562acaeeb547012b0447f3335279830405fafb37726532"
        );
        assert_eq!(
            typed_evidence_digest(&evidence),
            "3f649bd386b953b42442e8cefcbd1449d657f49a972f11d72f810bcda167756a"
        );
        assert_eq!(
            opaque_occurrence_key(
                &"0".repeat(64),
                &fingerprint,
                "test.coordination-gate.v1",
                "event-a",
            ),
            "occ:v1:760fc151831a9d5bf11893e402fdf5d63727e188dbc17015c67b2054f4a97148"
        );
    }

    #[test]
    fn producer_registry_has_unique_revisioned_entries() {
        let mut public_ids = std::collections::HashSet::new();
        let mut producer_ids = std::collections::HashSet::new();
        for registration in REGISTERED_PRODUCERS {
            assert!(public_ids.insert(registration.public_id));
            assert!(producer_ids.insert(registration.producer_id));
            assert!(registration.contract_schema_revision > 0);
            assert!(registration.routing_basis_revision > 0);
            assert!(is_contract_artifact(registration.target_artifact));
        }
    }

    #[test]
    fn registered_capture_automatically_creates_owner_after_capture_status_ack() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        env.owner_client
            .seed_repository_issue(gwt_github::client::RepositoryIssue {
                repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                number: gwt_github::client::IssueNumber(77),
                title: "Unrelated public issue".to_string(),
                body: "No improvement fingerprint".to_string(),
                labels: Vec::new(),
                state: gwt_github::client::IssueState::Open,
                kind: gwt_github::client::RepositoryIssueKind::Plain,
                updated_at: gwt_github::client::UpdatedAt::new("u77"),
            });
        let token = registered_producer_token("test.coordination-gate").expect("registered token");

        let candidate = capture_registered(
            &mut env,
            registered_capture_input(token, "auto-create", 1, CaptureBudgetProfile::Normal),
        )
        .expect("registered capture");

        assert_eq!(
            candidate.state,
            CandidateState::Created,
            "candidate: {candidate:?}"
        );
        assert_eq!(candidate.owner.as_ref().unwrap().number, 78);
        assert_eq!(candidate.linked_issue.as_ref().unwrap().number, 78);
        let stored = load_store(&env.repo_path).expect("stored candidate");
        assert_eq!(stored.candidates[0].state, CandidateState::Created);
        assert_eq!(stored.candidates[0].owner.as_ref().unwrap().number, 78);
        let mutations = env.owner_client.owner_mutation_call_log();
        assert_eq!(mutations.len(), 1);
        assert_eq!(
            mutations[0].operation,
            gwt_github::client::fake::OwnerRepositoryOperation::CreateIssue
        );
        let bodies = board_bodies(&mut env);
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0].contains("was captured"));
        assert!(bodies[1].contains("was created"));
    }

    #[test]
    fn registered_capture_preserves_new_owner_mutation_certainty() {
        use gwt_github::client::fake::{OwnerRepositoryFaultTiming, OwnerRepositoryOperation};

        for (timing, expected_state, expected_mutations) in [
            (
                OwnerRepositoryFaultTiming::BeforeSubmit,
                CandidateState::Blocked,
                0,
            ),
            (
                OwnerRepositoryFaultTiming::AfterSubmit,
                CandidateState::RemoteOutcomeUnknown,
                1,
            ),
        ] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let project = tempfile::tempdir().expect("project");
            let mut env = TestEnv::new(project.path().join("cache"));
            env.repo_path = project.path().join("source");
            std::fs::create_dir_all(&env.repo_path).expect("repo path");
            env.improvement_source_scope_nonce =
                crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                    .expect("canonical source scope nonce");
            env.owner_client
                .seed_repository_issue(gwt_github::client::RepositoryIssue {
                    repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                    number: gwt_github::client::IssueNumber(77),
                    title: "Unrelated public issue".to_string(),
                    body: "No improvement fingerprint".to_string(),
                    labels: Vec::new(),
                    state: gwt_github::client::IssueState::Open,
                    kind: gwt_github::client::RepositoryIssueKind::Plain,
                    updated_at: gwt_github::client::UpdatedAt::new("u77"),
                });
            env.owner_client.fail_next_owner_operation(
                OwnerRepositoryOperation::CreateIssue,
                timing,
                gwt_github::client::ApiError::Timeout {
                    operation: "create proven-zero owner".to_string(),
                },
            );
            let token =
                registered_producer_token("test.coordination-gate").expect("registered token");
            let input = registered_capture_input(
                token,
                "create-certainty",
                1,
                CaptureBudgetProfile::Normal,
            );

            let candidate = capture_registered(&mut env, input.clone())
                .expect("mutation certainty must settle durably");

            assert_eq!(
                candidate.state, expected_state,
                "timing: {timing:?}, candidate: {candidate:?}"
            );
            assert_eq!(
                env.owner_client.owner_mutation_count(),
                expected_mutations,
                "timing: {timing:?}"
            );
            let stored = load_store(&env.repo_path).expect("stored candidate");
            assert_eq!(stored.candidates[0].state, expected_state);

            if timing == OwnerRepositoryFaultTiming::AfterSubmit {
                let replay = capture_registered(&mut env, input)
                    .expect("remote-unknown replay must not mutate");
                assert_eq!(replay.state, CandidateState::RemoteOutcomeUnknown);
                assert_eq!(env.owner_client.owner_mutation_count(), 1);
                let adopted =
                    resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                        .expect("authoritative retry must adopt the created owner");
                assert_eq!(adopted.state, CandidateState::Created);
                assert_eq!(adopted.owner.as_ref().unwrap().number, 78);
                assert_eq!(env.owner_client.owner_mutation_count(), 1);
            }
        }
    }

    #[test]
    fn new_owner_local_commit_failure_preserves_create_intent_for_adoption() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let candidate =
            capture_registered_candidate_without_resolution(&mut env, "local-commit-failure");
        env.owner_client
            .seed_repository_issue(gwt_github::client::RepositoryIssue {
                repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                number: gwt_github::client::IssueNumber(77),
                title: "Unrelated public issue".to_string(),
                body: "No improvement fingerprint".to_string(),
                labels: Vec::new(),
                state: gwt_github::client::IssueState::Open,
                kind: gwt_github::client::RepositoryIssueKind::Plain,
                updated_at: gwt_github::client::UpdatedAt::new("u77"),
            });
        crate::cli::improvement_store::fail_next_owner_projection_commit()
            .expect("projection commit failure injection");

        let unknown =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("local commit failure must settle durably");

        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(unknown.blocked_reason, None);
        assert!(matches!(
            unknown.attempt.as_ref().expect("submitted attempt").intent,
            crate::cli::improvement_store::ResolutionAttemptIntent::CreateIssue { .. }
        ));
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        let adopted =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("retry must adopt the read-back created owner");

        assert_eq!(adopted.state, CandidateState::Created);
        assert_eq!(adopted.owner.as_ref().unwrap().number, 78);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn remote_unknown_refresh_failure_preserves_unknown_intent_without_mutation() {
        use gwt_github::client::fake::{OwnerRepositoryFaultTiming, OwnerRepositoryOperation};

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        env.owner_client
            .seed_repository_issue(gwt_github::client::RepositoryIssue {
                repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                number: gwt_github::client::IssueNumber(77),
                title: "Unrelated public issue".to_string(),
                body: "No improvement fingerprint".to_string(),
                labels: Vec::new(),
                state: gwt_github::client::IssueState::Open,
                kind: gwt_github::client::RepositoryIssueKind::Plain,
                updated_at: gwt_github::client::UpdatedAt::new("u77"),
            });
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            gwt_github::client::ApiError::Timeout {
                operation: "create proven-zero owner".to_string(),
            },
        );
        let token = registered_producer_token("test.coordination-gate").expect("registered token");
        let unknown = capture_registered(
            &mut env,
            registered_capture_input(token, "unknown-refresh", 1, CaptureBudgetProfile::Normal),
        )
        .expect("capture must preserve unknown");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::ListIssues,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            gwt_github::client::ApiError::Timeout {
                operation: "refresh unknown owner".to_string(),
            },
        );

        let retried = resolve_candidate_owner(&mut env, &unknown.id, CaptureBudgetProfile::Normal)
            .expect("refresh failure must settle durably");

        assert_eq!(retried.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
        let attempt = retried.attempt.expect("submitted recovery intent");
        assert_eq!(
            attempt.remote_phase,
            crate::cli::improvement_store::AttemptRemotePhase::Submitted
        );
        assert!(matches!(
            attempt.intent,
            crate::cli::improvement_store::ResolutionAttemptIntent::CreateIssue { .. }
        ));
    }

    #[test]
    fn remote_unknown_create_adoption_survives_a_new_occurrence() {
        use gwt_github::client::fake::{OwnerRepositoryFaultTiming, OwnerRepositoryOperation};

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        env.owner_client
            .seed_repository_issue(gwt_github::client::RepositoryIssue {
                repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                number: gwt_github::client::IssueNumber(77),
                title: "Unrelated public issue".to_string(),
                body: "No improvement fingerprint".to_string(),
                labels: Vec::new(),
                state: gwt_github::client::IssueState::Open,
                kind: gwt_github::client::RepositoryIssueKind::Plain,
                updated_at: gwt_github::client::UpdatedAt::new("u77"),
            });
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            gwt_github::client::ApiError::Timeout {
                operation: "create owner before recapture".to_string(),
            },
        );
        let producer =
            registered_producer_token("test.coordination-gate").expect("registered token");
        let unknown = capture_registered(
            &mut env,
            registered_capture_input(
                producer,
                "unknown-occurrence-a",
                1,
                CaptureBudgetProfile::Normal,
            ),
        )
        .expect("unknown create outcome");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);

        let mut changed_outcome = registered_capture_input(
            producer,
            "unknown-occurrence-b",
            1,
            CaptureBudgetProfile::Normal,
        );
        changed_outcome.expected_outcome = "BOARD_STATUS_RESTORED".to_string();
        changed_outcome.observed_outcome = "BOARD_STATUS_DELAYED".to_string();
        let recaptured = capture_registered(&mut env, changed_outcome)
            .expect("new occurrence while outcome is unknown");
        assert_eq!(recaptured.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(recaptured.occurrences, 2);

        let adopted = resolve_candidate_owner(&mut env, &unknown.id, CaptureBudgetProfile::Normal)
            .expect("retry must adopt the exact created marker");

        assert_eq!(adopted.state, CandidateState::Created);
        assert_eq!(adopted.owner.as_ref().unwrap().number, 78);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn legacy_unassigned_remote_intent_never_authorizes_zero_owner_create() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let candidate = capture_registered_candidate_without_resolution(&mut env, "legacy-unknown");
        env.owner_client
            .seed_repository_issue(gwt_github::client::RepositoryIssue {
                repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                number: gwt_github::client::IssueNumber(77),
                title: "Unrelated public issue".to_string(),
                body: "No improvement fingerprint".to_string(),
                labels: Vec::new(),
                state: gwt_github::client::IssueState::Open,
                kind: gwt_github::client::RepositoryIssueKind::Plain,
                updated_at: gwt_github::client::UpdatedAt::new("u77"),
            });
        crate::cli::improvement_store::update(&env.repo_path, |store| {
            let stored = find_candidate_mut(store, &candidate.id)?;
            let now = Utc::now();
            stored.retry = Some(RetryMetadata {
                retryable: true,
                remediation: "REFRESH_OWNER_CORPUS".to_string(),
                failed_at: now.to_rfc3339(),
            });
            transition_candidate(stored, CandidateState::RemoteOutcomeUnknown)?;
            stored.attempt = Some(crate::cli::improvement_store::ResolutionAttemptLease {
                attempt_id: "legacy-unassigned".to_string(),
                lease_owner: "legacy-worker".to_string(),
                started_at: now - chrono::Duration::seconds(30),
                expires_at: now - chrono::Duration::seconds(1),
                remote_phase: crate::cli::improvement_store::AttemptRemotePhase::Submitted,
                intent: crate::cli::improvement_store::ResolutionAttemptIntent::Unassigned,
            });
            Ok(())
        })
        .expect("seed legacy unknown attempt");

        let retried =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("legacy unknown retry");

        assert_eq!(retried.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(env.owner_client.owner_mutation_count(), 0);
        assert!(matches!(
            retried.attempt.expect("unknown attempt").intent,
            crate::cli::improvement_store::ResolutionAttemptIntent::Unassigned
        ));
    }

    #[test]
    fn resolver_rechecks_authoritative_corpus_before_create_and_adopts_visible_owner() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let evidence = TypedFailureEvidence {
            subsystem: "coordination".to_string(),
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            target_artifact: "coordination".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            observed_outcome: "BOARD_STATUS_MISSING".to_string(),
        };
        let command = ImprovementCaptureCommand {
            source: "hook-runtime".to_string(),
            target_artifact: "coordination".to_string(),
            classification: "gwt-caused".to_string(),
            confidence: "high".to_string(),
            summary: "Local deterministic capture context".to_string(),
            details: None,
            evidence_digest: None,
            dedupe_key: None,
            local_evidence: Vec::new(),
            typed_evidence: None,
        };
        let candidate = capture_typed(
            &env.repo_path,
            &command,
            evidence,
            ValidatedCaptureOrigin::Registered {
                producer_id: "test.coordination-gate.v1",
                source_event_id: "pre-create-race".to_string(),
                producer_registry_revision: PRODUCER_REGISTRY_REVISION,
                routing_basis_revision: 1,
                recurrence: None,
            },
        )
        .expect("capture candidate")
        .candidate;
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        let payload = render_public_issue_payload(
            &candidate,
            &PublicMutationContext::for_repo(&env.repo_path),
        )
        .expect("typed owner payload");
        let repository = gwt_github::client::RepositoryIdentity::gwt_upstream();
        let unrelated = gwt_github::client::RepositoryIssue {
            repository: repository.clone(),
            number: gwt_github::client::IssueNumber(77),
            title: "Unrelated public issue".to_string(),
            body: "No improvement fingerprint".to_string(),
            labels: Vec::new(),
            state: gwt_github::client::IssueState::Open,
            kind: gwt_github::client::RepositoryIssueKind::Plain,
            updated_at: gwt_github::client::UpdatedAt::new("u77"),
        };
        let visible_owner = gwt_github::client::RepositoryIssue {
            repository: repository.clone(),
            number: gwt_github::client::IssueNumber(78),
            title: payload.title,
            body: payload.body,
            labels: Vec::new(),
            state: gwt_github::client::IssueState::Open,
            kind: gwt_github::client::RepositoryIssueKind::Plain,
            updated_at: gwt_github::client::UpdatedAt::new("u78"),
        };
        env.owner_client
            .seed_repository_issue(visible_owner.clone());
        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![unrelated.clone()], "before"),
                (vec![unrelated.clone()], "before"),
                (vec![unrelated.clone(), visible_owner.clone()], "after"),
                (vec![unrelated, visible_owner], "after"),
            ],
        );

        let resolved =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("second scan must adopt the owner");

        assert_eq!(resolved.state, CandidateState::Linked);
        assert_eq!(resolved.owner.as_ref().unwrap().number, 78);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn registered_capture_automatically_links_revision_pinned_active_owner() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        seed_revision_pinned_active_owner(&mut env);
        let token = registered_producer_token("test.owner-route").expect("registered token");

        let candidate = capture_registered(
            &mut env,
            registered_capture_input(token, "auto-link", 9, CaptureBudgetProfile::Normal),
        )
        .expect("registered capture");

        assert_eq!(candidate.state, CandidateState::Linked);
        assert_eq!(candidate.owner.as_ref().unwrap().number, 77);
        assert_eq!(candidate.linked_issue.as_ref().unwrap().number, 77);
        let stored = load_store(&env.repo_path).expect("stored candidate");
        assert_eq!(stored.candidates[0].state, CandidateState::Linked);
        assert_eq!(stored.candidates[0].owner.as_ref().unwrap().number, 77);
        let mutations = env.owner_client.owner_mutation_call_log();
        assert_eq!(mutations.len(), 1);
        assert_eq!(
            mutations[0].operation,
            gwt_github::client::fake::OwnerRepositoryOperation::CreateComment
        );
        let bodies = board_bodies(&mut env);
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0].contains("was captured"));
        assert!(bodies[1].contains("was linked"));
    }

    #[test]
    fn resolver_reconciles_exact_plain_issue_duplicates_to_lowest_owner() {
        use gwt_github::client::{
            IssueNumber, IssueState, OwnerRepositoryClient, RepositoryIdentity, RepositoryIssue,
            RepositoryIssueKind, ResolutionDeadline, UpdatedAt,
        };
        use std::time::Duration;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        let fingerprint = improvement_fingerprint(&TypedFailureEvidence {
            subsystem: "coordination".to_string(),
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            target_artifact: "coordination".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            observed_outcome: "BOARD_STATUS_MISSING".to_string(),
        });
        let repository = RepositoryIdentity::gwt_upstream();
        for number in [78, 79] {
            env.owner_client.seed_repository_issue(RepositoryIssue {
                repository: repository.clone(),
                number: IssueNumber(number),
                title: format!("Duplicate owner {number}"),
                body: format!("<!-- gwt:improvement-fingerprint:v1 {fingerprint} -->"),
                labels: Vec::new(),
                state: IssueState::Open,
                kind: RepositoryIssueKind::Plain,
                updated_at: UpdatedAt::new(format!("u{number}")),
            });
        }
        let token = registered_producer_token("test.coordination-gate").expect("registered token");

        let resolved = capture_registered(
            &mut env,
            registered_capture_input(token, "duplicate-race", 1, CaptureBudgetProfile::Normal),
        )
        .expect("duplicates must reconcile");

        assert_eq!(resolved.state, CandidateState::Linked);
        assert_eq!(resolved.owner.as_ref().unwrap().number, 78);
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));
        let duplicate = env
            .owner_client
            .fetch_issue(&repository, IssueNumber(79), &deadline)
            .expect("duplicate readback");
        assert_eq!(duplicate.state, IssueState::Closed);
        let duplicate_comments = env
            .owner_client
            .list_comments(&repository, IssueNumber(79), &deadline)
            .expect("duplicate comments");
        assert!(duplicate_comments.items().iter().any(|comment| {
            comment
                .body
                .contains("gwt:improvement-reconciliation:v1 canonical:78 duplicate:79")
        }));
        let mutations = env.owner_client.owner_mutation_call_log();
        assert_eq!(
            mutations
                .iter()
                .filter(|call| {
                    call.operation
                        == gwt_github::client::fake::OwnerRepositoryOperation::CreateIssue
                })
                .count(),
            0
        );
        assert_eq!(mutations.len(), 3, "reconcile comment, close, occurrence");
    }

    #[test]
    fn independent_machine_stores_converge_after_delayed_duplicate_visibility() {
        use gwt_github::client::{
            fake::FakeIssueClient, IssueNumber, IssueState, OwnerRepositoryClient,
            RepositoryIdentity, RepositoryIssue, RepositoryIssueKind, ResolutionDeadline,
            UpdatedAt,
        };
        use std::time::Duration;

        let home_a = tempfile::tempdir().expect("machine A home");
        let home_b = tempfile::tempdir().expect("machine B home");
        let project = tempfile::tempdir().expect("source repository");
        let repository = RepositoryIdentity::gwt_upstream();
        let shared_owner = FakeIssueClient::new();
        let unrelated = RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(77),
            title: "Unrelated public issue".to_string(),
            body: "No improvement fingerprint".to_string(),
            labels: Vec::new(),
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new("u77"),
        };
        shared_owner.seed_repository_issue(unrelated.clone());
        let mut env_a = TestEnv::new(project.path().join("cache-a"));
        env_a.repo_path = project.path().join("source-a");
        std::fs::create_dir_all(&env_a.repo_path).expect("machine A source");
        env_a.owner_client = shared_owner.clone();
        let mut env_b = TestEnv::new(project.path().join("cache-b"));
        env_b.repo_path = project.path().join("source-b");
        std::fs::create_dir_all(&env_b.repo_path).expect("machine B source");
        env_b.owner_client = shared_owner.clone();

        let candidate_a = {
            let _home = gwt_core::test_support::ScopedGwtHome::set(home_a.path());
            let candidate =
                capture_registered_candidate_without_resolution(&mut env_a, "machine-a");
            let resolved =
                resolve_candidate_owner(&mut env_a, &candidate.id, CaptureBudgetProfile::Normal)
                    .expect("machine A create");
            assert_eq!(resolved.state, CandidateState::Created);
            assert_eq!(resolved.owner.as_ref().unwrap().number, 78);
            candidate
        };

        let resolved_b = {
            let _home = gwt_core::test_support::ScopedGwtHome::set(home_b.path());
            let candidate_b =
                capture_registered_candidate_without_resolution(&mut env_b, "machine-b");
            let payload = render_public_issue_payload(
                &candidate_b,
                &PublicMutationContext::for_repo(&env_b.repo_path),
            )
            .expect("machine B payload");
            let predicted_79 = RepositoryIssue {
                repository: repository.clone(),
                number: IssueNumber(79),
                title: payload.title,
                body: payload.body,
                labels: Vec::new(),
                state: IssueState::Open,
                kind: RepositoryIssueKind::Plain,
                updated_at: UpdatedAt::new("predicted-u79"),
            };
            let hidden = vec![unrelated.clone()];
            let machine_b_only = vec![unrelated.clone(), predicted_79];
            shared_owner.queue_owner_issue_views(
                &repository,
                [
                    (hidden.clone(), "machine-b-zero-1"),
                    (hidden.clone(), "machine-b-zero-1"),
                    (hidden.clone(), "machine-b-zero-2"),
                    (hidden, "machine-b-zero-2"),
                    (machine_b_only.clone(), "machine-b-only"),
                    (machine_b_only, "machine-b-only"),
                ],
            );

            let initially_created =
                resolve_candidate_owner(&mut env_b, &candidate_b.id, CaptureBudgetProfile::Normal)
                    .expect("machine B create");
            assert_eq!(initially_created.state, CandidateState::Created);
            assert_eq!(initially_created.owner.as_ref().unwrap().number, 79);

            resolve_candidate_owner(&mut env_b, &candidate_b.id, CaptureBudgetProfile::Normal)
                .expect("machine B convergence audit")
        };

        assert_eq!(resolved_b.state, CandidateState::Linked);
        assert_eq!(resolved_b.owner.as_ref().unwrap().number, 78);
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));
        assert_eq!(
            shared_owner
                .fetch_issue(&repository, IssueNumber(79), &deadline)
                .expect("duplicate owner")
                .state,
            IssueState::Closed
        );
        assert!(shared_owner
            .fetch_issue(&repository, IssueNumber(80), &deadline)
            .is_err());
        assert_eq!(
            shared_owner
                .owner_mutation_call_log()
                .iter()
                .filter(|call| {
                    call.operation
                        == gwt_github::client::fake::OwnerRepositoryOperation::CreateIssue
                })
                .count(),
            2
        );
        {
            let _home = gwt_core::test_support::ScopedGwtHome::set(home_a.path());
            let stored_a = load_store(&env_a.repo_path).expect("machine A store");
            let stored_a = stored_a
                .candidates
                .iter()
                .find(|candidate| candidate.id == candidate_a.id)
                .expect("machine A candidate");
            assert_eq!(stored_a.owner.as_ref().unwrap().number, 78);
        }
    }

    #[test]
    fn reconciliation_failure_stops_then_retry_reuses_exact_comment() {
        use gwt_github::client::fake::{OwnerRepositoryFaultTiming, OwnerRepositoryOperation};
        use gwt_github::client::{
            IssueNumber, OwnerRepositoryClient, RepositoryIdentity, ResolutionDeadline,
        };
        use std::time::Duration;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        seed_exact_plain_owners(&mut env, &[78, 79]);
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CloseIssue,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            gwt_github::client::ApiError::PermissionDenied {
                message: "close duplicate".to_string(),
            },
        );
        let producer =
            registered_producer_token("test.coordination-gate").expect("registered token");

        let blocked = capture_registered(
            &mut env,
            registered_capture_input(
                producer,
                "duplicate-close-failure",
                1,
                CaptureBudgetProfile::Normal,
            ),
        )
        .expect("known reconciliation failure must settle");

        assert_eq!(blocked.state, CandidateState::Blocked);
        assert_eq!(blocked.blocked_reason, Some(BlockedReason::Reconciliation));
        assert!(blocked.reconciliation_required);
        assert_eq!(blocked.reconciliation_owner_numbers, vec![78, 79]);
        let mutations = env.owner_client.owner_mutation_call_log();
        assert_eq!(mutations.len(), 1, "must stop before close and occurrence");
        assert_eq!(
            mutations[0].operation,
            OwnerRepositoryOperation::CreateComment
        );

        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::ListIssues,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            gwt_github::client::ApiError::Network(
                "temporary duplicate-corpus refresh failure".to_string(),
            ),
        );
        let refresh_blocked =
            resolve_candidate_owner(&mut env, &blocked.id, CaptureBudgetProfile::Normal)
                .expect("temporary reconciliation refresh failure must settle");
        assert_eq!(refresh_blocked.state, CandidateState::Blocked);
        assert_eq!(
            refresh_blocked.blocked_reason,
            Some(BlockedReason::Reconciliation),
            "a transient refresh failure must not clear the no-create reconciliation latch"
        );
        assert!(refresh_blocked.reconciliation_required);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        super::super::improvement_store::update(&env.repo_path, |store| {
            let candidate = find_candidate_mut(store, &blocked.id)?;
            candidate.state = CandidateState::OwnerResolving;
            candidate.blocked_reason = None;
            candidate.failure_subcode = None;
            candidate.retry = None;
            candidate.attempt = Some(super::super::improvement_store::ResolutionAttemptLease {
                attempt_id: "crashed-reconciliation-attempt".to_string(),
                lease_owner: "crashed-worker".to_string(),
                started_at: Utc::now() - chrono::Duration::minutes(2),
                expires_at: Utc::now() - chrono::Duration::minutes(1),
                remote_phase: super::super::improvement_store::AttemptRemotePhase::NotSubmitted,
                intent: super::super::improvement_store::ResolutionAttemptIntent::Unassigned,
            });
            validate_candidate_lifecycle(candidate)
        })
        .expect("simulate crash after reconciliation attempt acquisition");

        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));
        let canonical = env
            .owner_client
            .fetch_issue(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(78),
                &deadline,
            )
            .expect("canonical owner");
        env.owner_client.queue_owner_issue_views(
            &RepositoryIdentity::gwt_upstream(),
            [
                (vec![canonical.clone()], "canonical-only"),
                (vec![canonical], "canonical-only"),
            ],
        );
        let cleanup_blocked =
            resolve_candidate_owner(&mut env, &blocked.id, CaptureBudgetProfile::Normal)
                .expect("hidden duplicate cleanup must settle");
        assert_eq!(cleanup_blocked.state, CandidateState::Blocked);
        assert_eq!(
            cleanup_blocked.blocked_reason,
            Some(BlockedReason::Reconciliation),
            "canonical visibility alone must not prove duplicate cleanup"
        );
        assert!(cleanup_blocked.reconciliation_required);
        assert_eq!(cleanup_blocked.reconciliation_owner_numbers, vec![78, 79]);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        let repository = RepositoryIdentity::gwt_upstream();
        let unrelated = gwt_github::client::RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(77),
            title: "Temporarily visible unrelated Issue".to_string(),
            body: "No matching fingerprint".to_string(),
            labels: Vec::new(),
            state: gwt_github::client::IssueState::Open,
            kind: gwt_github::client::RepositoryIssueKind::Plain,
            updated_at: gwt_github::client::UpdatedAt::new("u77"),
        };
        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![unrelated.clone()], "hidden-1"),
                (vec![unrelated], "hidden-1"),
            ],
        );
        let still_blocked =
            resolve_candidate_owner(&mut env, &blocked.id, CaptureBudgetProfile::Normal)
                .expect("reconciliation-only retry must settle");
        assert_eq!(still_blocked.state, CandidateState::Blocked);
        assert_eq!(
            still_blocked.blocked_reason,
            Some(BlockedReason::Reconciliation)
        );
        assert!(still_blocked.reconciliation_required);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        let linked = resolve_candidate_owner(&mut env, &blocked.id, CaptureBudgetProfile::Normal)
            .expect("retry reconciliation");

        assert_eq!(linked.state, CandidateState::Linked);
        assert_eq!(linked.owner.as_ref().unwrap().number, 78);
        assert!(!linked.reconciliation_required);
        assert!(linked.reconciliation_owner_numbers.is_empty());
        let duplicate_comments = env
            .owner_client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(79),
                &deadline,
            )
            .expect("duplicate comments");
        assert_eq!(
            duplicate_comments
                .items()
                .iter()
                .filter(|comment| comment.body.contains("gwt:improvement-reconciliation:v1"))
                .count(),
            1,
            "retry must not duplicate the immutable reconciliation marker"
        );
        assert_eq!(env.owner_client.owner_mutation_count(), 3);
        assert!(env
            .owner_client
            .owner_mutation_call_log()
            .iter()
            .all(|call| call.operation != OwnerRepositoryOperation::CreateIssue));
    }

    #[test]
    fn reconciliation_post_submit_unknown_adopts_comment_before_close_retry() {
        use gwt_github::client::fake::{OwnerRepositoryFaultTiming, OwnerRepositoryOperation};
        use gwt_github::client::{
            IssueNumber, OwnerRepositoryClient, RepositoryIdentity, ResolutionDeadline,
        };
        use std::time::Duration;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        seed_exact_plain_owners(&mut env, &[78, 79]);
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateComment,
            OwnerRepositoryFaultTiming::AfterSubmit,
            gwt_github::client::ApiError::Timeout {
                operation: "reconciliation comment response".to_string(),
            },
        );
        let producer =
            registered_producer_token("test.coordination-gate").expect("registered token");

        let unknown = capture_registered(
            &mut env,
            registered_capture_input(
                producer,
                "duplicate-comment-unknown",
                1,
                CaptureBudgetProfile::Normal,
            ),
        )
        .expect("unknown reconciliation must settle");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        let linked = resolve_candidate_owner(&mut env, &unknown.id, CaptureBudgetProfile::Normal)
            .expect("authoritative comment adoption");

        assert_eq!(linked.state, CandidateState::Linked);
        assert_eq!(linked.owner.as_ref().unwrap().number, 78);
        assert_eq!(env.owner_client.owner_mutation_count(), 3);
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));
        let comments = env
            .owner_client
            .list_comments(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(79),
                &deadline,
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
    fn reconciliation_post_submit_unknown_waits_for_authoritative_comment_marker() {
        use gwt_github::client::fake::{OwnerRepositoryFaultTiming, OwnerRepositoryOperation};
        use gwt_github::client::{IssueNumber, RepositoryIdentity};

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        seed_exact_plain_owners(&mut env, &[78, 79]);
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateComment,
            OwnerRepositoryFaultTiming::AfterSubmit,
            gwt_github::client::ApiError::Timeout {
                operation: "reconciliation comment response".to_string(),
            },
        );
        let producer =
            registered_producer_token("test.coordination-gate").expect("registered token");

        let unknown = capture_registered(
            &mut env,
            registered_capture_input(
                producer,
                "duplicate-comment-delayed-readback",
                1,
                CaptureBudgetProfile::Normal,
            ),
        )
        .expect("unknown reconciliation must settle");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        env.owner_client.seed_repository_comments(
            &RepositoryIdentity::gwt_upstream(),
            IssueNumber(79),
            Vec::new(),
        );
        let still_unknown =
            resolve_candidate_owner(&mut env, &unknown.id, CaptureBudgetProfile::Normal)
                .expect("missing authoritative marker must remain unknown");

        assert_eq!(still_unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);
    }

    #[test]
    fn occurrence_comment_unknown_rebinds_to_lower_canonical_owner_after_readback() {
        use gwt_github::client::fake::{OwnerRepositoryFaultTiming, OwnerRepositoryOperation};
        use gwt_github::client::{
            IssueNumber, IssueState, OwnerRepositoryClient, RepositoryIdentity, ResolutionDeadline,
        };
        use std::time::Duration;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let candidate =
            capture_registered_candidate_without_resolution(&mut env, "occurrence-owner-race");
        seed_exact_plain_owners(&mut env, &[79]);
        env.owner_client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateComment,
            OwnerRepositoryFaultTiming::AfterSubmit,
            gwt_github::client::ApiError::Timeout {
                operation: "occurrence comment response".to_string(),
            },
        );

        let unknown =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("unknown occurrence comment must settle");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        let repository = RepositoryIdentity::gwt_upstream();
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));
        let submitted_comments = env
            .owner_client
            .list_comments(&repository, IssueNumber(79), &deadline)
            .expect("submitted occurrence comment")
            .items()
            .to_vec();
        env.owner_client
            .seed_repository_comments(&repository, IssueNumber(79), Vec::new());
        let still_unknown =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("missing occurrence marker must remain unknown");
        assert_eq!(still_unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(env.owner_client.owner_mutation_count(), 1);

        env.owner_client
            .seed_repository_comments(&repository, IssueNumber(79), submitted_comments);
        seed_exact_plain_owners(&mut env, &[78]);
        let linked = resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
            .expect("verified losing-owner comment must rebind to canonical owner");

        assert_eq!(linked.state, CandidateState::Linked);
        assert_eq!(linked.owner.as_ref().map(|owner| owner.number), Some(78));
        assert_eq!(env.owner_client.owner_mutation_count(), 4);
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(79), &deadline)
                .expect("losing owner readback")
                .state,
            IssueState::Closed
        );
        for number in [78, 79] {
            let comments = env
                .owner_client
                .list_comments(&repository, IssueNumber(number), &deadline)
                .expect("owner comments");
            assert_eq!(
                comments
                    .items()
                    .iter()
                    .filter(|comment| comment.body.contains("gwt:improvement-occurrence:v1"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn registered_capture_preserves_active_owner_mutation_certainty() {
        use gwt_github::client::fake::{OwnerRepositoryFaultTiming, OwnerRepositoryOperation};

        for (timing, expected_state, expected_mutations) in [
            (
                OwnerRepositoryFaultTiming::BeforeSubmit,
                CandidateState::Blocked,
                0,
            ),
            (
                OwnerRepositoryFaultTiming::AfterSubmit,
                CandidateState::RemoteOutcomeUnknown,
                1,
            ),
        ] {
            let home = tempfile::tempdir().expect("isolated home");
            let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
            let project = tempfile::tempdir().expect("project");
            let mut env = TestEnv::new(project.path().join("cache"));
            env.repo_path = project.path().join("source");
            std::fs::create_dir_all(&env.repo_path).expect("repo path");
            seed_revision_pinned_active_owner(&mut env);
            env.owner_client.fail_next_owner_operation(
                OwnerRepositoryOperation::CreateComment,
                timing,
                gwt_github::client::ApiError::Timeout {
                    operation: "create active owner occurrence".to_string(),
                },
            );
            let token = registered_producer_token("test.owner-route").expect("registered token");
            let input =
                registered_capture_input(token, "certainty", 9, CaptureBudgetProfile::Normal);

            let candidate = capture_registered(&mut env, input.clone())
                .expect("mutation certainty must settle durably");

            assert_eq!(candidate.state, expected_state, "timing: {timing:?}");
            assert_eq!(
                env.owner_client.owner_mutation_count(),
                expected_mutations,
                "timing: {timing:?}"
            );
            let stored = load_store(&env.repo_path).expect("stored candidate");
            assert_eq!(stored.candidates[0].state, expected_state);

            if timing == OwnerRepositoryFaultTiming::AfterSubmit {
                let replay = capture_registered(&mut env, input)
                    .expect("remote-unknown replay must not mutate");
                assert_eq!(replay.state, CandidateState::RemoteOutcomeUnknown);
                assert_eq!(env.owner_client.owner_mutation_count(), 1);
                let adopted =
                    resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                        .expect("authoritative retry must adopt the occurrence comment");
                assert_eq!(adopted.state, CandidateState::Linked, "{adopted:?}");
                assert_eq!(adopted.owner.as_ref().unwrap().number, 77);
                assert_eq!(env.owner_client.owner_mutation_count(), 1);
            }
        }
    }

    #[test]
    fn resolver_honors_live_lease_and_takes_over_expired_unsubmitted_lease() {
        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        let token = registered_producer_token("test.coordination-gate").expect("registered token");
        let blocked = capture_registered(
            &mut env,
            registered_capture_input(token, "lease", 1, CaptureBudgetProfile::Normal),
        )
        .expect("initial blocked capture");
        assert_eq!(blocked.state, CandidateState::Blocked);
        let initial_owner_calls = env.owner_client.owner_call_log().len();
        let now = Utc::now();
        crate::cli::improvement_store::update(&env.repo_path, |store| {
            let candidate = find_candidate_mut(store, &blocked.id)?;
            transition_candidate(candidate, CandidateState::OwnerResolving)?;
            candidate.attempt = Some(crate::cli::improvement_store::ResolutionAttemptLease {
                attempt_id: "attempt-live".to_string(),
                lease_owner: "other-worker".to_string(),
                started_at: now,
                expires_at: now + chrono::Duration::seconds(30),
                remote_phase: crate::cli::improvement_store::AttemptRemotePhase::NotSubmitted,
                intent: crate::cli::improvement_store::ResolutionAttemptIntent::Unassigned,
            });
            Ok(())
        })
        .expect("seed live lease");

        let error = resolve_candidate_owner(&mut env, &blocked.id, CaptureBudgetProfile::Normal)
            .expect_err("live lease must block duplicate local resolution");

        assert!(error.to_string().contains("already in progress"));
        assert_eq!(env.owner_client.owner_call_log().len(), initial_owner_calls);

        crate::cli::improvement_store::update(&env.repo_path, |store| {
            let candidate = find_candidate_mut(store, &blocked.id)?;
            candidate.attempt.as_mut().unwrap().expires_at =
                Utc::now() - chrono::Duration::seconds(1);
            Ok(())
        })
        .expect("expire lease");
        let retried = resolve_candidate_owner(&mut env, &blocked.id, CaptureBudgetProfile::Normal)
            .expect("expired unsubmitted lease must be taken over");
        assert_eq!(retried.state, CandidateState::Blocked);
        assert!(retried.attempt.is_none());
        assert!(env.owner_client.owner_call_log().len() > initial_owner_calls);
    }

    #[test]
    fn promote_force_is_removed_before_candidate_lookup_or_owner_transport() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let error = run_collect(
            &mut env,
            CliCommand::Improvement(ImprovementCommand::PromoteIssue(
                ImprovementPromoteIssueCommand {
                    id: "missing-candidate".to_string(),
                    force: true,
                    labels: Vec::new(),
                },
            )),
        )
        .expect_err("force must be removed before candidate lookup");

        assert!(error.to_string().contains("UNSAFE_FORCE_REMOVED"));
        assert!(env.owner_client.owner_call_log().is_empty());
        assert!(env.owner_client.owner_mutation_call_log().is_empty());
        assert!(env.target_issue_create_call_log.is_empty());
    }

    #[test]
    fn registered_capture_qualifies_once_and_replay_does_not_duplicate_status() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source-a");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        let token = registered_producer_token("test.coordination-gate").expect("registered token");

        let first = capture_registered(
            &mut env,
            registered_capture_input(token, "event-a", 1, CaptureBudgetProfile::Normal),
        )
        .expect("registered capture");
        assert_eq!(first.eligibility, ImprovementEligibility::Deterministic);
        assert_eq!(first.state, CandidateState::Blocked);
        assert_eq!(first.blocked_reason, Some(BlockedReason::Search));
        assert_eq!(first.failure_subcode, Some(FailureSubcode::EmptyCorpus));
        assert_eq!(first.occurrences, 1);
        let canonical_source_scope_nonce = load_store(&env.repo_path)
            .expect("candidate store")
            .source_scope_nonce
            .expect("source scope nonce");
        assert!(owner_eligibility_is_canonical(
            &first,
            &env.repo_path,
            &canonical_source_scope_nonce,
        ));
        let occurrence = &first.distinct_occurrences[0];
        assert!(occurrence.qualifies_unattended);
        assert_eq!(
            occurrence.producer_id.as_deref(),
            Some("test.coordination-gate.v1")
        );
        assert_eq!(occurrence.producer_registry_revision, Some(1));
        assert_eq!(occurrence.routing_basis_revision, Some(1));
        assert_eq!(board_bodies(&mut env).len(), 2);

        let replay = capture_registered(
            &mut env,
            registered_capture_input(token, "event-a", 1, CaptureBudgetProfile::Normal),
        )
        .expect("registered replay");
        assert_eq!(replay.id, first.id);
        assert_eq!(replay.occurrences, 1);
        assert_eq!(
            replay.distinct_occurrences[0].producer_id,
            occurrence.producer_id
        );
        assert_eq!(
            replay.distinct_occurrences[0].routing_basis_revision,
            occurrence.routing_basis_revision
        );
        assert_eq!(board_bodies(&mut env).len(), 2, "replay must stay silent");

        let mut conflicting =
            registered_capture_input(token, "event-a", 1, CaptureBudgetProfile::Normal);
        conflicting.observed_outcome = "BOARD_STATUS_POSTED_TOO_LATE".to_string();
        let error = capture_registered(&mut env, conflicting)
            .expect_err("conflicting deterministic replay must fail closed");
        assert!(error
            .to_string()
            .contains("conflicting improvement occurrence replay"));
        assert_eq!(
            load_store(&env.repo_path)
                .expect("store after conflict")
                .candidates[0]
                .occurrences,
            1
        );

        let second = capture_registered(
            &mut env,
            registered_capture_input(token, "event-b", 1, CaptureBudgetProfile::Normal),
        )
        .expect("second registered event");
        assert_eq!(second.id, first.id);
        assert_eq!(second.occurrences, 2);
        assert_eq!(second.state, CandidateState::Blocked);
        assert_eq!(board_bodies(&mut env).len(), 4);
    }

    #[test]
    fn distinct_public_outcomes_with_one_fingerprint_remain_owner_eligible() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let first = capture_registered_candidate_without_resolution(&mut env, "outcome-event-a");
        let mut evidence = first.typed_evidence.clone().expect("typed evidence");
        evidence.expected_outcome = "BOARD_STATUS_RESTORED".to_string();
        evidence.observed_outcome = "BOARD_STATUS_DELAYED".to_string();
        let command = ImprovementCaptureCommand {
            source: "hook-runtime".to_string(),
            target_artifact: "coordination".to_string(),
            classification: "gwt-caused".to_string(),
            confidence: "high".to_string(),
            summary: "Local deterministic capture context".to_string(),
            details: None,
            evidence_digest: None,
            dedupe_key: None,
            local_evidence: Vec::new(),
            typed_evidence: None,
        };

        let second = capture_typed(
            &env.repo_path,
            &command,
            evidence,
            ValidatedCaptureOrigin::Registered {
                producer_id: "test.coordination-gate.v1",
                source_event_id: "outcome-event-b".to_string(),
                producer_registry_revision: PRODUCER_REGISTRY_REVISION,
                routing_basis_revision: 1,
                recurrence: None,
            },
        )
        .expect("second public outcome")
        .candidate;

        assert_eq!(second.id, first.id);
        assert_eq!(second.fingerprint, first.fingerprint);
        assert_eq!(second.occurrences, 2);
        assert!(owner_eligibility_is_canonical(
            &second,
            &env.repo_path,
            &env.improvement_source_scope_nonce,
        ));
    }

    #[test]
    fn registered_recurrence_is_occurrence_scoped_digest_bound_and_capability_gated() {
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        let token = registered_producer_token("test.coordination-gate").expect("registered token");
        let recurrence = TypedRecurrenceEvidence {
            installed_version: Some("9.66.0".to_string()),
            build_commit: None,
            observed_at: "2026-07-15T09:00:00Z".to_string(),
        };
        let mut input =
            registered_capture_input(token, "recurrence-a", 1, CaptureBudgetProfile::Normal);
        input.recurrence = Some(recurrence.clone());

        let first = capture_registered(&mut env, input.clone()).expect("registered recurrence");
        let occurrence = &first.distinct_occurrences[0];
        assert_eq!(occurrence.recurrence.as_ref(), Some(&recurrence));
        assert_ne!(
            occurrence.evidence_digest,
            typed_evidence_digest(first.typed_evidence.as_ref().expect("typed evidence"))
        );
        assert!(owner_eligibility_is_canonical(
            &first,
            &env.repo_path,
            &env.improvement_source_scope_nonce,
        ));

        let replay = capture_registered(&mut env, input).expect("identical recurrence replay");
        assert_eq!(replay.occurrences, 1);

        let mut conflicting =
            registered_capture_input(token, "recurrence-a", 1, CaptureBudgetProfile::Normal);
        conflicting.recurrence = Some(TypedRecurrenceEvidence {
            installed_version: Some("9.67.0".to_string()),
            build_commit: None,
            observed_at: "2026-07-15T09:00:00Z".to_string(),
        });
        let error = capture_registered(&mut env, conflicting)
            .expect_err("same event cannot replace recurrence proof");
        assert!(error
            .to_string()
            .contains("conflicting improvement occurrence replay"));

        let other_project = tempfile::tempdir().expect("other project");
        let mut other_env = TestEnv::new(other_project.path().join("cache"));
        other_env.repo_path = other_project.path().join("source");
        std::fs::create_dir_all(&other_env.repo_path).expect("other repo path");
        let non_capable = registered_producer_token("test.owner-route").expect("registered token");
        let mut rejected =
            registered_capture_input(non_capable, "recurrence-b", 9, CaptureBudgetProfile::Normal);
        rejected.recurrence = Some(recurrence);
        let error = capture_registered(&mut other_env, rejected)
            .expect_err("producer without recurrence capability must be rejected");
        assert!(error.to_string().contains("recurrence-capable"));
        assert!(
            crate::cli::improvement_store::load_and_repair(&other_env.repo_path)
                .expect("empty store")
                .candidates
                .is_empty()
        );
    }

    #[test]
    fn registered_capture_scopes_same_event_by_random_source_nonce() {
        let project = tempfile::tempdir().expect("project");
        let token = registered_producer_token("test.coordination-gate").expect("registered token");
        let mut env_a = TestEnv::new(project.path().join("cache-a"));
        env_a.repo_path = project.path().join("source-a");
        let mut env_b = TestEnv::new(project.path().join("cache-b"));
        env_b.repo_path = project.path().join("source-b");
        std::fs::create_dir_all(&env_a.repo_path).expect("repo a");
        std::fs::create_dir_all(&env_b.repo_path).expect("repo b");

        let candidate_a = capture_registered(
            &mut env_a,
            registered_capture_input(token, "same-event", 1, CaptureBudgetProfile::Normal),
        )
        .expect("capture a");
        let candidate_b = capture_registered(
            &mut env_b,
            registered_capture_input(token, "same-event", 1, CaptureBudgetProfile::Normal),
        )
        .expect("capture b");
        let store_a = load_store(&env_a.repo_path).expect("store a");
        let store_b = load_store(&env_b.repo_path).expect("store b");

        assert_ne!(store_a.source_scope_nonce, store_b.source_scope_nonce);
        assert_ne!(
            candidate_a.distinct_occurrences[0].opaque_key,
            candidate_b.distinct_occurrences[0].opaque_key
        );
        let replay_a = capture_registered(
            &mut env_a,
            registered_capture_input(token, "same-event", 1, CaptureBudgetProfile::Normal),
        )
        .expect("replay a");
        assert_eq!(replay_a.occurrences, 1);
    }

    #[test]
    fn registered_eligibility_is_not_reclassified_by_public_capture() {
        let project = tempfile::tempdir().expect("project");
        let token = registered_producer_token("test.coordination-gate").expect("registered token");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");

        let mut ineligible_input =
            registered_capture_input(token, "low-event", 1, CaptureBudgetProfile::Normal);
        ineligible_input.classification = "external".to_string();
        ineligible_input.confidence = "low".to_string();
        let ineligible =
            capture_registered(&mut env, ineligible_input).expect("ineligible registered capture");
        assert_eq!(ineligible.eligibility, ImprovementEligibility::Ineligible);
        assert_eq!(ineligible.state, CandidateState::NeedsEvidence);

        let public_command = ImprovementCaptureCommand {
            source: "agent-failure".to_string(),
            target_artifact: "coordination".to_string(),
            classification: "gwt-caused".to_string(),
            confidence: "high".to_string(),
            summary: "Unverified public report".to_string(),
            details: None,
            evidence_digest: None,
            dedupe_key: None,
            local_evidence: Vec::new(),
            typed_evidence: None,
        };
        let evidence = validate_typed_evidence(
            &ImprovementTypedEvidenceCommand {
                subsystem: "coordination".to_string(),
                contract_id: "coordination.board-status".to_string(),
                contract_schema_revision: 1,
                failure_code: "STATUS_NOT_POSTED".to_string(),
                expected_outcome: "BOARD_STATUS_POSTED".to_string(),
                observed_outcome: "BOARD_STATUS_MISSING".to_string(),
            },
            "coordination",
        )
        .expect("typed evidence");
        let after_public = capture_typed(
            &env.repo_path,
            &public_command,
            evidence,
            ValidatedCaptureOrigin::Interpretive { session_id: None },
        )
        .expect("public capture")
        .candidate;
        assert_eq!(after_public.eligibility, ImprovementEligibility::Ineligible);
        assert_eq!(after_public.state, CandidateState::NeedsEvidence);
        assert_eq!(after_public.occurrences, 1);

        let mut env = TestEnv::new(project.path().join("cache-eligible"));
        env.repo_path = project.path().join("source-eligible");
        std::fs::create_dir_all(&env.repo_path).expect("eligible repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        let deterministic = capture_registered(
            &mut env,
            registered_capture_input(token, "high-event", 1, CaptureBudgetProfile::Normal),
        )
        .expect("eligible registered capture");
        assert_eq!(
            deterministic.eligibility,
            ImprovementEligibility::Deterministic
        );
        let mut low_public = public_command;
        low_public.classification = "external".to_string();
        low_public.confidence = "low".to_string();
        let evidence = validate_typed_evidence(
            &ImprovementTypedEvidenceCommand {
                subsystem: "coordination".to_string(),
                contract_id: "coordination.board-status".to_string(),
                contract_schema_revision: 1,
                failure_code: "STATUS_NOT_POSTED".to_string(),
                expected_outcome: "BOARD_STATUS_POSTED".to_string(),
                observed_outcome: "BOARD_STATUS_MISSING".to_string(),
            },
            "coordination",
        )
        .expect("typed evidence");
        let after_low_public = capture_typed(
            &env.repo_path,
            &low_public,
            evidence,
            ValidatedCaptureOrigin::Interpretive { session_id: None },
        )
        .expect("low public capture")
        .candidate;
        assert_eq!(
            after_low_public.eligibility,
            ImprovementEligibility::Deterministic
        );
        assert_eq!(after_low_public.state, CandidateState::Blocked);
    }

    #[test]
    fn registered_occurrence_identity_binds_the_fingerprint() {
        let project = tempfile::tempdir().expect("project");
        let token = registered_producer_token("test.coordination-gate").expect("registered token");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");

        let first = capture_registered(
            &mut env,
            registered_capture_input(token, "same-event", 1, CaptureBudgetProfile::Normal),
        )
        .expect("first fingerprint");
        let mut second_input =
            registered_capture_input(token, "same-event", 1, CaptureBudgetProfile::Normal);
        second_input.failure_code = "STATUS_CONTENT_INVALID".to_string();
        let second = capture_registered(&mut env, second_input).expect("second fingerprint");

        assert_ne!(first.fingerprint, second.fingerprint);
        assert_ne!(
            first.distinct_occurrences[0].opaque_key,
            second.distinct_occurrences[0].opaque_key
        );
    }

    #[cfg(unix)]
    #[test]
    fn registered_capture_retries_pending_board_status_after_post_failure() {
        use std::os::unix::fs::PermissionsExt;

        let project = tempfile::tempdir().expect("project");
        let token = registered_producer_token("test.coordination-gate").expect("registered token");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        std::fs::set_permissions(&env.repo_path, std::fs::Permissions::from_mode(0o500))
            .expect("make Board path read-only");

        let first = capture_registered(
            &mut env,
            registered_capture_input(token, "board-retry", 1, CaptureBudgetProfile::Normal),
        );
        std::fs::set_permissions(&env.repo_path, std::fs::Permissions::from_mode(0o700))
            .expect("restore Board path permissions");
        assert!(first.is_err(), "first Board post must fail");

        capture_registered(
            &mut env,
            registered_capture_input(token, "board-retry", 1, CaptureBudgetProfile::Normal),
        )
        .expect("retry capture");
        let bodies = board_bodies(&mut env);
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0].contains("was updated"));
        assert!(bodies[1].contains("is blocked"));
    }

    #[cfg(unix)]
    #[test]
    fn resolver_persists_blocked_state_before_failure_status_post() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        let evidence = TypedFailureEvidence {
            subsystem: "coordination".to_string(),
            contract_id: "coordination.board-status".to_string(),
            contract_schema_revision: 1,
            failure_code: "STATUS_NOT_POSTED".to_string(),
            target_artifact: "coordination".to_string(),
            expected_outcome: "BOARD_STATUS_POSTED".to_string(),
            observed_outcome: "BOARD_STATUS_MISSING".to_string(),
        };
        let command = ImprovementCaptureCommand {
            source: "hook-runtime".to_string(),
            target_artifact: "coordination".to_string(),
            classification: "gwt-caused".to_string(),
            confidence: "high".to_string(),
            summary: "Local deterministic capture context".to_string(),
            details: None,
            evidence_digest: None,
            dedupe_key: None,
            local_evidence: Vec::new(),
            typed_evidence: None,
        };
        let captured = capture_typed(
            &env.repo_path,
            &command,
            evidence,
            ValidatedCaptureOrigin::Registered {
                producer_id: "test.coordination-gate.v1",
                source_event_id: "blocked-board".to_string(),
                producer_registry_revision: 1,
                routing_basis_revision: 1,
                recurrence: None,
            },
        )
        .expect("registered capture without status delivery")
        .candidate;
        assert_eq!(captured.state, CandidateState::OwnerResolving);

        let coordination_dir = gwt_core::coordination::coordination_dir(&env.repo_path);
        std::fs::create_dir_all(&coordination_dir).expect("coordination directory");
        std::fs::set_permissions(&coordination_dir, std::fs::Permissions::from_mode(0o500))
            .expect("make Board store read-only");
        let result = resolve_candidate_owner(&mut env, &captured.id, CaptureBudgetProfile::Normal);
        std::fs::set_permissions(&coordination_dir, std::fs::Permissions::from_mode(0o700))
            .expect("restore Board store permissions");

        assert!(result.is_err(), "failure status post must surface");
        let stored = load_store(&env.repo_path).expect("stored blocked candidate");
        assert_eq!(stored.candidates[0].state, CandidateState::Blocked);
        assert_eq!(
            stored.candidates[0].blocked_reason,
            Some(BlockedReason::Search)
        );
        assert_eq!(
            stored.candidates[0].failure_subcode,
            Some(FailureSubcode::EmptyCorpus)
        );
    }

    #[cfg(unix)]
    #[test]
    fn owner_success_board_status_retries_after_durable_commit() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.owner_client
            .seed_repository_issue(gwt_github::client::RepositoryIssue {
                repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                number: gwt_github::client::IssueNumber(77),
                title: "Unrelated public issue".to_string(),
                body: "No improvement fingerprint".to_string(),
                labels: Vec::new(),
                state: gwt_github::client::IssueState::Open,
                kind: gwt_github::client::RepositoryIssueKind::Plain,
                updated_at: gwt_github::client::UpdatedAt::new("u77"),
            });
        let captured =
            capture_registered_candidate_without_resolution(&mut env, "owner-board-retry");
        let coordination_dir = gwt_core::coordination::coordination_dir(&env.repo_path);
        std::fs::create_dir_all(&coordination_dir).expect("coordination directory");
        std::fs::set_permissions(&coordination_dir, std::fs::Permissions::from_mode(0o500))
            .expect("make Board store read-only");

        let first = resolve_candidate_owner(&mut env, &captured.id, CaptureBudgetProfile::Normal);

        std::fs::set_permissions(&coordination_dir, std::fs::Permissions::from_mode(0o700))
            .expect("restore Board store permissions");
        assert!(
            first.is_err(),
            "the failed success-status post must surface"
        );
        let stored = load_store(&env.repo_path).expect("stored created candidate");
        let stored = stored
            .candidates
            .iter()
            .find(|candidate| candidate.id == captured.id)
            .expect("created candidate");
        assert_eq!(stored.state, CandidateState::Created);
        assert_eq!(stored.owner.as_ref().unwrap().number, 78);
        assert_eq!(stored.owner_status_generation, 1);
        assert_eq!(stored.owner_status_delivered_generation, 0);
        let recurrent =
            capture_registered_candidate_without_resolution(&mut env, "owner-board-retry-next");
        assert_eq!(recurrent.id, captured.id);
        assert_eq!(recurrent.state, CandidateState::Recurrent);
        assert_eq!(recurrent.occurrences, 2);

        let retried = resolve_candidate_owner(&mut env, &captured.id, CaptureBudgetProfile::Normal)
            .expect("retry pending success status");

        assert_eq!(retried.state, CandidateState::Created);
        assert_eq!(retried.owner.as_ref().unwrap().number, 78);
        assert_eq!(retried.occurrences, 2);
        assert_eq!(
            retried.owner_status_delivered_generation,
            retried.owner_status_generation
        );
        let bodies = board_bodies(&mut env);
        assert_eq!(
            bodies
                .iter()
                .filter(|body| body.contains("owner #78 was created"))
                .count(),
            1,
            "the durable created status must be delivered exactly once"
        );
        assert!(bodies
            .iter()
            .all(|body| !body.contains("was linked to active")));
        assert_eq!(
            env.owner_client
                .owner_mutation_call_log()
                .iter()
                .filter(|call| {
                    call.operation
                        == gwt_github::client::fake::OwnerRepositoryOperation::CreateIssue
                })
                .count(),
            1,
            "status retry must not create another owner"
        );
    }

    #[cfg(unix)]
    #[test]
    fn same_event_replay_delivers_pending_owner_success_status() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.owner_client
            .seed_repository_issue(gwt_github::client::RepositoryIssue {
                repository: gwt_github::client::RepositoryIdentity::gwt_upstream(),
                number: gwt_github::client::IssueNumber(77),
                title: "Unrelated public issue".to_string(),
                body: "No improvement fingerprint".to_string(),
                labels: Vec::new(),
                state: gwt_github::client::IssueState::Open,
                kind: gwt_github::client::RepositoryIssueKind::Plain,
                updated_at: gwt_github::client::UpdatedAt::new("u77"),
            });
        let captured =
            capture_registered_candidate_without_resolution(&mut env, "owner-board-same-event");
        let coordination_dir = gwt_core::coordination::coordination_dir(&env.repo_path);
        std::fs::create_dir_all(&coordination_dir).expect("coordination directory");
        std::fs::set_permissions(&coordination_dir, std::fs::Permissions::from_mode(0o500))
            .expect("make Board store read-only");

        let first = resolve_candidate_owner(&mut env, &captured.id, CaptureBudgetProfile::Normal);

        std::fs::set_permissions(&coordination_dir, std::fs::Permissions::from_mode(0o700))
            .expect("restore Board store permissions");
        assert!(
            first.is_err(),
            "the failed success-status post must surface"
        );
        let producer =
            registered_producer_token("test.coordination-gate").expect("registered token");
        let replay = capture_registered(
            &mut env,
            registered_capture_input(
                producer,
                "owner-board-same-event",
                1,
                CaptureBudgetProfile::Normal,
            ),
        )
        .expect("same event replay");

        assert_eq!(replay.state, CandidateState::Created);
        assert_eq!(replay.owner.as_ref().unwrap().number, 78);
        assert_eq!(replay.occurrences, 1);
        assert_eq!(
            replay.owner_status_delivered_generation, replay.owner_status_generation,
            "same-event retry must drain the durable owner status"
        );
        assert_eq!(
            board_bodies(&mut env)
                .iter()
                .filter(|body| body.contains("owner #78 was created"))
                .count(),
            1
        );
        assert_eq!(
            env.owner_client
                .owner_mutation_call_log()
                .iter()
                .filter(|call| {
                    call.operation
                        == gwt_github::client::fake::OwnerRepositoryOperation::CreateIssue
                })
                .count(),
            1,
            "same-event status retry must not create another owner"
        );
    }

    #[test]
    fn successful_candidate_stable_zero_never_creates_replacement_owner() {
        use gwt_github::client::{IssueNumber, OwnerRepositoryClient, RepositoryIdentity};

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        env.improvement_source_scope_nonce =
            crate::cli::improvement_store::source_scope_nonce(&env.repo_path)
                .expect("canonical source scope nonce");
        env.owner_client
            .seed_repository_issue(gwt_github::client::RepositoryIssue {
                repository: RepositoryIdentity::gwt_upstream(),
                number: IssueNumber(77),
                title: "Unrelated public issue".to_string(),
                body: "No improvement fingerprint".to_string(),
                labels: Vec::new(),
                state: gwt_github::client::IssueState::Open,
                kind: gwt_github::client::RepositoryIssueKind::Plain,
                updated_at: gwt_github::client::UpdatedAt::new("u77"),
            });
        let producer =
            registered_producer_token("test.coordination-gate").expect("registered token");
        let created = capture_registered(
            &mut env,
            registered_capture_input(
                producer,
                "known-owner-stable-zero",
                1,
                CaptureBudgetProfile::Normal,
            ),
        )
        .expect("initial owner creation");
        assert_eq!(created.state, CandidateState::Created);
        assert_eq!(created.owner.as_ref().unwrap().number, 78);
        let deadline = gwt_github::client::ResolutionDeadline::new(
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(5),
        );
        let unrelated = env
            .owner_client
            .fetch_issue(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(77),
                &deadline,
            )
            .expect("unrelated issue");
        env.owner_client.queue_owner_issue_views(
            &RepositoryIdentity::gwt_upstream(),
            [
                (vec![unrelated.clone()], "known-owner-hidden"),
                (vec![unrelated.clone()], "known-owner-hidden"),
                (vec![unrelated.clone()], "known-owner-hidden"),
                (vec![unrelated], "known-owner-hidden"),
            ],
        );

        let audited = resolve_candidate_owner(&mut env, &created.id, CaptureBudgetProfile::Normal)
            .expect("known-owner zero must settle fail-closed");

        assert_eq!(audited.state, CandidateState::Blocked);
        assert_eq!(audited.blocked_reason, Some(BlockedReason::Readback));
        assert_eq!(audited.owner.as_ref().unwrap().number, 78);
        assert_eq!(
            env.owner_client
                .owner_mutation_call_log()
                .iter()
                .filter(|call| {
                    call.operation
                        == gwt_github::client::fake::OwnerRepositoryOperation::CreateIssue
                })
                .count(),
            1,
            "stable zero must not replace an already durable owner"
        );

        let still_hidden =
            resolve_candidate_owner(&mut env, &created.id, CaptureBudgetProfile::Normal)
                .expect("second stable-zero audit");
        assert_eq!(still_hidden.state, CandidateState::Blocked);
        assert_eq!(still_hidden.owner.as_ref().unwrap().number, 78);
        let recovered =
            resolve_candidate_owner(&mut env, &created.id, CaptureBudgetProfile::Normal)
                .expect("visible durable owner retry");
        assert!(matches!(
            recovered.state,
            CandidateState::Linked | CandidateState::Created
        ));
        assert_eq!(recovered.owner.as_ref().unwrap().number, 78);
        assert_eq!(
            env.owner_client
                .owner_mutation_call_log()
                .iter()
                .filter(|call| {
                    call.operation
                        == gwt_github::client::fake::OwnerRepositoryOperation::CreateIssue
                })
                .count(),
            1
        );
    }

    #[test]
    fn post_create_stable_zero_recovers_the_readback_owner_without_a_second_create() {
        use gwt_github::client::{
            IssueNumber, OwnerRepositoryClient, RepositoryIdentity, RepositoryIssue,
        };

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let candidate =
            capture_registered_candidate_without_resolution(&mut env, "post-create-stable-zero");
        let repository = RepositoryIdentity::gwt_upstream();
        let unrelated = RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(77),
            title: "Unrelated public issue".to_string(),
            body: "No improvement fingerprint".to_string(),
            labels: Vec::new(),
            state: gwt_github::client::IssueState::Open,
            kind: gwt_github::client::RepositoryIssueKind::Plain,
            updated_at: gwt_github::client::UpdatedAt::new("u77"),
        };
        env.owner_client.seed_repository_issue(unrelated.clone());
        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![unrelated.clone()], "zero-1"),
                (vec![unrelated.clone()], "zero-1"),
                (vec![unrelated.clone()], "zero-2"),
                (vec![unrelated.clone()], "zero-2"),
                (vec![unrelated.clone()], "post-create-zero"),
                (vec![unrelated.clone()], "post-create-zero"),
            ],
        );

        let unknown =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("post-create zero must preserve recovery state");
        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert!(matches!(
            unknown.attempt.as_ref().map(|attempt| &attempt.intent),
            Some(
                crate::cli::improvement_store::ResolutionAttemptIntent::CreateIssue {
                    created_owner_number: Some(78),
                    ..
                }
            )
        ));
        assert_eq!(
            env.owner_client
                .owner_mutation_call_log()
                .iter()
                .filter(|call| {
                    call.operation
                        == gwt_github::client::fake::OwnerRepositoryOperation::CreateIssue
                })
                .count(),
            1
        );

        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![unrelated.clone()], "retry-zero-1"),
                (vec![unrelated.clone()], "retry-zero-1"),
                (vec![unrelated.clone()], "retry-zero-2"),
                (vec![unrelated.clone()], "retry-zero-2"),
                (vec![unrelated.clone()], "retry-post-create-zero"),
                (vec![unrelated], "retry-post-create-zero"),
            ],
        );
        let still_unknown =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("recorded readback owner must block another create");

        assert_eq!(still_unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert_eq!(
            env.owner_client
                .owner_mutation_call_log()
                .iter()
                .filter(|call| {
                    call.operation
                        == gwt_github::client::fake::OwnerRepositoryOperation::CreateIssue
                })
                .count(),
            1,
            "stable zero retry must not create a replacement owner"
        );
        assert!(env
            .owner_client
            .fetch_issue(
                &repository,
                IssueNumber(79),
                &gwt_github::client::ResolutionDeadline::new(
                    std::time::Duration::from_secs(1),
                    std::time::Duration::from_secs(5),
                ),
            )
            .is_err());
    }

    #[test]
    fn created_owner_number_survives_one_shot_source_save_failure() {
        use gwt_github::client::{
            IssueNumber, IssueState, OwnerRepositoryClient, RepositoryIdentity, RepositoryIssue,
            RepositoryIssueKind, ResolutionDeadline, UpdatedAt,
        };
        use std::time::Duration;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let candidate =
            capture_registered_candidate_without_resolution(&mut env, "numbered-save-failure");
        let payload = render_public_issue_payload(
            &candidate,
            &PublicMutationContext::for_repo(&env.repo_path),
        )
        .expect("owner payload");
        let repository = RepositoryIdentity::gwt_upstream();
        let unrelated = RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(77),
            title: "Unrelated public issue".to_string(),
            body: "No improvement fingerprint".to_string(),
            labels: Vec::new(),
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new("u77"),
        };
        let competing_owner = RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(78),
            title: payload.title,
            body: payload.body,
            labels: Vec::new(),
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new("u78"),
        };
        env.owner_client.seed_repository_issue(unrelated.clone());
        env.owner_client
            .seed_repository_issue(competing_owner.clone());
        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![unrelated.clone()], "zero-1"),
                (vec![unrelated.clone()], "zero-1"),
                (vec![unrelated.clone()], "zero-2"),
                (vec![unrelated], "zero-2"),
            ],
        );
        crate::cli::improvement_store::fail_next_created_owner_number_save(&env.repo_path)
            .expect("source save failure injection");

        let unknown =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("one-shot source save failure must settle");

        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert!(matches!(
            unknown.attempt.as_ref().map(|attempt| &attempt.intent),
            Some(
                crate::cli::improvement_store::ResolutionAttemptIntent::CreateIssue {
                    created_owner_number: Some(79),
                    ..
                }
            )
        ));
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(79), &deadline)
                .expect("created owner")
                .state,
            IssueState::Open
        );

        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![competing_owner.clone()], "competing-only"),
                (vec![competing_owner], "competing-only"),
            ],
        );
        let blocked =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("hidden created owner must remain in reconciliation");
        assert!(!matches!(
            blocked.state,
            CandidateState::Linked | CandidateState::Created
        ));
        assert!(blocked.reconciliation_required);
        assert_eq!(blocked.reconciliation_owner_numbers, vec![78, 79]);
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(79), &deadline)
                .expect("unsettled created owner")
                .state,
            IssueState::Open
        );
    }

    #[test]
    fn post_create_hidden_owner_preserves_the_complete_reconciliation_set() {
        use gwt_github::client::{
            IssueNumber, IssueState, OwnerRepositoryClient, RepositoryIdentity, RepositoryIssue,
            RepositoryIssueKind, ResolutionDeadline, UpdatedAt,
        };
        use std::time::Duration;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let candidate =
            capture_registered_candidate_without_resolution(&mut env, "hidden-created-owner");
        let payload = render_public_issue_payload(
            &candidate,
            &PublicMutationContext::for_repo(&env.repo_path),
        )
        .expect("owner payload");
        let repository = RepositoryIdentity::gwt_upstream();
        let unrelated = RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(77),
            title: "Unrelated public issue".to_string(),
            body: "No improvement fingerprint".to_string(),
            labels: Vec::new(),
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new("u77"),
        };
        let competing_owner = RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(78),
            title: payload.title,
            body: payload.body,
            labels: Vec::new(),
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new("u78"),
        };
        env.owner_client.seed_repository_issue(unrelated.clone());
        env.owner_client
            .seed_repository_issue(competing_owner.clone());
        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![unrelated.clone()], "zero-1"),
                (vec![unrelated.clone()], "zero-1"),
                (vec![unrelated.clone()], "zero-2"),
                (vec![unrelated], "zero-2"),
                (vec![competing_owner.clone()], "post-create-other"),
                (vec![competing_owner.clone()], "post-create-other"),
            ],
        );

        let unknown =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("post-create conflict must settle durably");

        assert_eq!(unknown.state, CandidateState::RemoteOutcomeUnknown);
        assert!(unknown.reconciliation_required);
        assert_eq!(unknown.reconciliation_owner_numbers, vec![78, 79]);
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(79), &deadline)
                .expect("hidden created owner")
                .state,
            IssueState::Open
        );

        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![competing_owner.clone()], "created-still-hidden"),
                (vec![competing_owner], "created-still-hidden"),
            ],
        );
        let still_unsettled =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("hidden known duplicate must remain unsettled");
        assert!(!matches!(
            still_unsettled.state,
            CandidateState::Linked | CandidateState::Created
        ));
        assert!(still_unsettled.reconciliation_required);
        assert_eq!(still_unsettled.reconciliation_owner_numbers, vec![78, 79]);
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(79), &deadline)
                .expect("still-open hidden owner")
                .state,
            IssueState::Open
        );

        let converged =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("fully visible duplicate set must converge");
        assert_eq!(converged.state, CandidateState::Linked);
        assert_eq!(converged.owner.as_ref().unwrap().number, 78);
        assert!(!converged.reconciliation_required);
        assert!(converged.reconciliation_owner_numbers.is_empty());
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(79), &deadline)
                .expect("reconciled duplicate")
                .state,
            IssueState::Closed
        );
    }

    #[test]
    fn post_create_duplicate_view_unions_the_hidden_created_owner() {
        use gwt_github::client::{
            IssueNumber, IssueState, OwnerRepositoryClient, RepositoryIdentity, RepositoryIssue,
            RepositoryIssueKind, ResolutionDeadline, UpdatedAt,
        };
        use std::time::Duration;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let candidate =
            capture_registered_candidate_without_resolution(&mut env, "hidden-created-duplicate");
        let payload = render_public_issue_payload(
            &candidate,
            &PublicMutationContext::for_repo(&env.repo_path),
        )
        .expect("owner payload");
        let repository = RepositoryIdentity::gwt_upstream();
        let unrelated = RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(77),
            title: "Unrelated public issue".to_string(),
            body: "No improvement fingerprint".to_string(),
            labels: Vec::new(),
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new("u77"),
        };
        let owner_78 = RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(78),
            title: payload.title.clone(),
            body: payload.body.clone(),
            labels: Vec::new(),
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new("u78"),
        };
        let owner_79 = RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(79),
            title: payload.title,
            body: payload.body,
            labels: Vec::new(),
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new("u79"),
        };
        for issue in [&unrelated, &owner_78, &owner_79] {
            env.owner_client.seed_repository_issue(issue.clone());
        }
        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![unrelated.clone()], "zero-1"),
                (vec![unrelated.clone()], "zero-1"),
                (vec![unrelated.clone()], "zero-2"),
                (vec![unrelated], "zero-2"),
                (
                    vec![owner_78.clone(), owner_79.clone()],
                    "visible-duplicates",
                ),
                (vec![owner_78.clone(), owner_79], "visible-duplicates"),
                (vec![owner_78.clone()], "canonical-only"),
                (vec![owner_78], "canonical-only"),
            ],
        );

        let unsettled =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("hidden created duplicate must settle fail-closed");

        assert!(!matches!(
            unsettled.state,
            CandidateState::Linked | CandidateState::Created
        ));
        assert!(unsettled.reconciliation_required);
        assert_eq!(unsettled.reconciliation_owner_numbers, vec![78, 79, 80]);
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(80), &deadline)
                .expect("hidden created owner")
                .state,
            IssueState::Open
        );

        let converged =
            resolve_candidate_owner(&mut env, &candidate.id, CaptureBudgetProfile::Normal)
                .expect("known hidden duplicate must converge once visible");
        assert_eq!(converged.state, CandidateState::Linked);
        assert_eq!(converged.owner.as_ref().unwrap().number, 78);
        assert!(!converged.reconciliation_required);
        assert!(converged.reconciliation_owner_numbers.is_empty());
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(80), &deadline)
                .expect("reconciled hidden owner")
                .state,
            IssueState::Closed
        );
    }

    #[test]
    fn visible_owner_conflicting_with_durable_owner_is_reconciled_before_rebind() {
        use gwt_github::client::{
            IssueNumber, IssueState, OwnerRepositoryClient, RepositoryIdentity, ResolutionDeadline,
        };
        use std::time::Duration;

        let home = tempfile::tempdir().expect("isolated home");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let project = tempfile::tempdir().expect("project");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        seed_exact_plain_owners(&mut env, &[79]);
        let producer =
            registered_producer_token("test.coordination-gate").expect("registered token");
        let linked = capture_registered(
            &mut env,
            registered_capture_input(
                producer,
                "durable-owner-conflict",
                1,
                CaptureBudgetProfile::Normal,
            ),
        )
        .expect("initial durable owner");
        assert_eq!(linked.state, CandidateState::Linked);
        assert_eq!(linked.owner.as_ref().unwrap().number, 79);

        seed_exact_plain_owners(&mut env, &[78]);
        let repository = RepositoryIdentity::gwt_upstream();
        let deadline = ResolutionDeadline::new(Duration::from_secs(1), Duration::from_secs(5));
        let lower_owner = env
            .owner_client
            .fetch_issue(&repository, IssueNumber(78), &deadline)
            .expect("lower owner");
        env.owner_client.queue_owner_issue_views(
            &repository,
            [
                (vec![lower_owner.clone()], "lower-only"),
                (vec![lower_owner], "lower-only"),
            ],
        );

        let reconciled =
            resolve_candidate_owner(&mut env, &linked.id, CaptureBudgetProfile::Normal)
                .expect("durable owner conflict must reconcile");

        assert_eq!(reconciled.state, CandidateState::Linked);
        assert_eq!(reconciled.owner.as_ref().unwrap().number, 78);
        assert!(!reconciled.reconciliation_required);
        assert!(reconciled.reconciliation_owner_numbers.is_empty());
        assert_eq!(
            env.owner_client
                .fetch_issue(&repository, IssueNumber(79), &deadline)
                .expect("former durable owner")
                .state,
            IssueState::Closed,
            "the previous durable owner must not remain an open duplicate"
        );
        assert!(env
            .owner_client
            .owner_mutation_call_log()
            .iter()
            .all(|call| call.operation
                != gwt_github::client::fake::OwnerRepositoryOperation::CreateIssue));
    }

    #[test]
    fn capture_status_ack_does_not_clear_a_newer_pending_generation() {
        let project = tempfile::tempdir().expect("project");
        let token = registered_producer_token("test.coordination-gate").expect("registered token");
        let mut env = TestEnv::new(project.path().join("cache"));
        env.repo_path = project.path().join("source");
        std::fs::create_dir_all(&env.repo_path).expect("repo path");
        let candidate = capture_registered(
            &mut env,
            registered_capture_input(token, "generation-1", 1, CaptureBudgetProfile::Normal),
        )
        .expect("initial delivered capture");

        super::super::improvement_store::update(&env.repo_path, |store| {
            let candidate = find_candidate_mut(store, &candidate.id)?;
            candidate.capture_status_generation = 3;
            candidate.capture_status_delivered_generation = 1;
            Ok(())
        })
        .expect("simulate two concurrent pending captures");

        acknowledge_capture_status(&env.repo_path, &candidate.id, 2)
            .expect("older successful post acknowledgement");
        let stored = load_store(&env.repo_path).expect("store after older acknowledgement");
        assert_eq!(stored.candidates[0].capture_status_delivered_generation, 2);
        assert_eq!(
            pending_capture_status_generation(&stored.candidates[0]),
            Some(3)
        );

        acknowledge_capture_status(&env.repo_path, &candidate.id, 3)
            .expect("newer post acknowledgement");
        let stored = load_store(&env.repo_path).expect("store after newer acknowledgement");
        assert_eq!(
            pending_capture_status_generation(&stored.candidates[0]),
            None
        );
    }

    #[test]
    fn registered_capture_rejects_unregistered_stale_or_disallowed_callers_before_mutation() {
        assert!(registered_producer_token("not-registered").is_err());

        let project = tempfile::tempdir().expect("project");
        let token = registered_producer_token("test.coordination-gate").expect("registered token");
        for (suffix, input) in [
            (
                "stale",
                registered_capture_input(token, "event", 0, CaptureBudgetProfile::Normal),
            ),
            (
                "budget",
                registered_capture_input(token, "event", 1, CaptureBudgetProfile::StrictStop),
            ),
            (
                "forged",
                registered_capture_input(
                    RegisteredProducerToken {
                        registry_revision: u64::MAX,
                        registration_index: usize::MAX,
                    },
                    "event",
                    1,
                    CaptureBudgetProfile::Normal,
                ),
            ),
            ("target", {
                let mut input =
                    registered_capture_input(token, "event", 1, CaptureBudgetProfile::Normal);
                input.target_artifact = "skill".to_string();
                input
            }),
        ] {
            let mut env = TestEnv::new(project.path().join(format!("cache-{suffix}")));
            env.repo_path = project.path().join(format!("source-{suffix}"));
            std::fs::create_dir_all(&env.repo_path).expect("repo path");
            assert!(capture_registered(&mut env, input).is_err());
            assert!(
                !super::super::improvement_store::candidate_store_path(&env.repo_path).exists(),
                "rejected {suffix} caller must not mutate the store"
            );
        }
    }
}
