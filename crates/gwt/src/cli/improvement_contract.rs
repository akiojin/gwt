use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

pub const OWNER_PROJECTION_CONTRACT_REVISION: u32 = 1;
pub const MANAGED_HOOK_PRODUCER_ID: &str = "managed-hook.failure";
pub const MANAGED_HOOK_GATE_ID: &str = "managed-hook.failure";
pub const MANAGED_HOOK_TARGET_ARTIFACT: &str = "issue-spec-workflow";
pub const MANAGED_HOOK_CONTRACT_REVISION: u64 = 1;
pub const MANAGED_HOOK_FAILURE_CODE: &str = "FRESH_INTAKE_OUTCOME_NOT_RECORDED";
pub const MANAGED_HOOK_EXPECTED_OUTCOME: &str = "FRESH_DURABLE_INTAKE_OUTCOME";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedHookEvidenceKind {
    Deterministic,
    Interpretive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedHookFailureEvent {
    pub producer: String,
    pub gate_id: String,
    pub failure_code: String,
    pub target_artifact: String,
    pub contract_revision: u64,
    pub session_key: String,
    pub event_key: String,
    pub expected_outcome: String,
    pub observed_outcome: String,
    pub evidence_kind: ManagedHookEvidenceKind,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedHookEligibility {
    Deterministic,
    InterpretiveCorroboration,
    NeedsEvidence,
    Ineligible,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedHookCaptureResult {
    pub candidate_id: String,
    pub fingerprint: String,
    pub occurrences: u64,
    pub eligibility: ManagedHookEligibility,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerProjectionSnapshot {
    pub contract_revision: u32,
    pub owners: Vec<OwnerProjectionAggregate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerProjectionAggregate {
    pub owner: OwnerProjectionOwner,
    pub fingerprint: String,
    pub aggregate_count: u64,
    pub last_seen: String,
    pub occurrences: Vec<OwnerProjectionOccurrence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerProjectionOwner {
    pub number: u64,
    pub kind: OwnerProjectionOwnerKind,
    pub active: bool,
    pub title: String,
    pub url: String,
    pub readback_verified_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum OwnerProjectionOwnerKind {
    Issue,
    Spec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnerProjectionOccurrence {
    pub opaque_key: String,
    pub public_marker_digest: String,
    pub last_seen: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerProjectionReadError {
    message: String,
}

impl OwnerProjectionReadError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for OwnerProjectionReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for OwnerProjectionReadError {}

pub fn read_owner_projection() -> Result<OwnerProjectionSnapshot, OwnerProjectionReadError> {
    super::improvement_store::read_owner_projection_contract()
        .map_err(|error| OwnerProjectionReadError::new(error.to_string()))
}

pub fn capture_managed_hook_failure<E: super::CliEnv>(
    env: &mut E,
    event: ManagedHookFailureEvent,
) -> Result<ManagedHookCaptureResult, gwt_github::SpecOpsError> {
    super::improvement::capture_managed_hook_failure(env, event)
}

pub fn managed_hook_failure_fingerprint(
    event: &ManagedHookFailureEvent,
) -> Result<String, gwt_github::SpecOpsError> {
    super::improvement::managed_hook_failure_fingerprint(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::TestEnv;
    use gwt_core::test_support::ScopedGwtHome;

    fn managed_hook_event(
        evidence_kind: ManagedHookEvidenceKind,
        session_key: &str,
        event_key: &str,
    ) -> ManagedHookFailureEvent {
        let mut event = ManagedHookFailureEvent {
            producer: MANAGED_HOOK_PRODUCER_ID.to_string(),
            gate_id: MANAGED_HOOK_GATE_ID.to_string(),
            failure_code: MANAGED_HOOK_FAILURE_CODE.to_string(),
            target_artifact: MANAGED_HOOK_TARGET_ARTIFACT.to_string(),
            contract_revision: MANAGED_HOOK_CONTRACT_REVISION,
            session_key: session_key.to_string(),
            event_key: event_key.to_string(),
            expected_outcome: MANAGED_HOOK_EXPECTED_OUTCOME.to_string(),
            observed_outcome: "DURABLE_INTAKE_OUTCOME_MISSING".to_string(),
            evidence_kind,
            fingerprint: String::new(),
        };
        event.fingerprint =
            managed_hook_failure_fingerprint(&event).expect("canonical managed-hook fingerprint");
        event
    }

    fn save_verified_session(repo: &std::path::Path, branch: &str) -> String {
        let mut session = gwt_agent::Session::new(repo, branch, gwt_agent::AgentId::Codex);
        session.repo_hash = Some(gwt_core::paths::project_scope_hash(repo).to_string());
        let id = session.id.clone();
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save verified session");
        id
    }

    #[test]
    fn managed_hook_producer_deduplicates_fingerprint_and_rejects_unknown_producer() {
        let home = tempfile::tempdir().expect("home");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let repo = tempfile::tempdir().expect("repo");
        let mut env = TestEnv::new(repo.path().join("cache"));
        env.repo_path = repo.path().to_path_buf();
        env.improvement_source_scope_nonce =
            super::super::improvement_store::source_scope_nonce(repo.path())
                .expect("source scope nonce");
        let session = save_verified_session(repo.path(), "work/managed-hook");
        let event = managed_hook_event(
            ManagedHookEvidenceKind::Deterministic,
            &session,
            "managed-hook-event-a",
        );

        let first =
            capture_managed_hook_failure(&mut env, event.clone()).expect("first typed capture");
        let replay =
            capture_managed_hook_failure(&mut env, event.clone()).expect("idempotent replay");
        assert_eq!(replay.candidate_id, first.candidate_id);
        assert_eq!(replay.fingerprint, first.fingerprint);
        assert_eq!(replay.occurrences, 1);
        assert_eq!(replay.eligibility, ManagedHookEligibility::Deterministic);
        let candidates = crate::cli::improvement::candidate_public_values(repo.path());
        assert_eq!(candidates[0]["state"], "blocked");
        assert_eq!(
            candidates[0]["blocked_reason"], "search",
            "the empty fake corpus must be reached through bounded Owner Resolution"
        );
        let (connect_timeout, total_remaining) = env
            .last_owner_client_budget()
            .expect("managed Stop producer must invoke Owner Resolution");
        assert!(connect_timeout <= std::time::Duration::from_secs(3));
        assert!(total_remaining <= std::time::Duration::from_secs(15));

        let mut forged = event.clone();
        forged.fingerprint = format!("{}0", forged.fingerprint);
        let error = capture_managed_hook_failure(&mut env, forged)
            .expect_err("forged fingerprint must be rejected");
        assert!(error.to_string().contains("not canonical"), "{error}");

        let mut unknown = event;
        unknown.producer = "unknown.managed-hook".to_string();
        let error = capture_managed_hook_failure(&mut env, unknown)
            .expect_err("unknown producer must be rejected");
        assert!(error.to_string().contains("not registered"), "{error}");
    }

    #[test]
    fn managed_hook_eligibility_requires_two_distinct_interpretive_sessions() {
        let home = tempfile::tempdir().expect("home");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let repo = tempfile::tempdir().expect("repo");
        let mut env = TestEnv::new(repo.path().join("cache"));
        env.repo_path = repo.path().to_path_buf();
        env.improvement_source_scope_nonce =
            super::super::improvement_store::source_scope_nonce(repo.path())
                .expect("source scope nonce");
        let session_a = save_verified_session(repo.path(), "work/session-a");
        let session_b = save_verified_session(repo.path(), "work/session-b");

        let first = capture_managed_hook_failure(
            &mut env,
            managed_hook_event(
                ManagedHookEvidenceKind::Interpretive,
                &session_a,
                "interpretive-a",
            ),
        )
        .expect("first interpretive occurrence");
        assert_eq!(first.eligibility, ManagedHookEligibility::NeedsEvidence);
        assert_eq!(first.occurrences, 1);

        let same_session = capture_managed_hook_failure(
            &mut env,
            managed_hook_event(
                ManagedHookEvidenceKind::Interpretive,
                &session_a,
                "interpretive-a-retry",
            ),
        )
        .expect("same-session replay");
        assert_eq!(
            same_session.eligibility,
            ManagedHookEligibility::NeedsEvidence
        );
        assert_eq!(same_session.occurrences, 1);

        let corroborated = capture_managed_hook_failure(
            &mut env,
            managed_hook_event(
                ManagedHookEvidenceKind::Interpretive,
                &session_b,
                "interpretive-b",
            ),
        )
        .expect("distinct-session corroboration");
        assert_eq!(
            corroborated.eligibility,
            ManagedHookEligibility::InterpretiveCorroboration
        );
        assert_eq!(corroborated.occurrences, 2);
    }
}
