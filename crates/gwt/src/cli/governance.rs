//! Pure governance value types shared by operation-local evaluators.
//!
//! This module deliberately owns no operation registry, I/O, or durable
//! state. Each operation remains responsible for its own outcome and probe.

use serde::{Deserialize, Serialize};

use gwt_core::board_escalation::OperationRefusalKind;
use gwt_github::SpecOpsError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceEffect {
    Observe,
    Reversible,
    Protected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceCause {
    StructuralGovernance,
    TransientGovernance,
    ExternalWait,
    NotReady,
    Authority,
    Integrity,
    ManagedIdentity,
    DomainInvalid,
}

impl GovernanceCause {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::StructuralGovernance => "structural_governance",
            Self::TransientGovernance => "transient_governance",
            Self::ExternalWait => "external_wait",
            Self::NotReady => "not_ready",
            Self::Authority => "authority",
            Self::Integrity => "integrity",
            Self::ManagedIdentity => "managed_identity",
            Self::DomainInvalid => "domain_invalid",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GovernanceMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<GovernanceEffect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<GovernanceCause>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_generation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_id: Option<String>,
}

/// Whether the refusing caller can reach the next valid action itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalRecoverability {
    AgentRecoverable,
    HumanRequired,
}

/// Stable, operation-local refusal facts carried alongside human output.
///
/// `reason_code` and the typed disposition decide escalation. `output` stays
/// outside this DTO and remains display-only, so wording changes cannot alter
/// control flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRefusal {
    pub reason_code: String,
    pub recoverability: RefusalRecoverability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<String>,
    /// Issue that owns the refused work, when the refusing operation knows it
    /// from durable state rather than from the caller's Session.
    ///
    /// An escalation without an owner reaches neither the Issue nor
    /// `needs_human`, and the refusals that most need an owner — a missing or
    /// unreadable Session identity — are exactly the ones where the Session
    /// cannot supply it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_number: Option<u64>,
    #[serde(default, skip_serializing_if = "metadata_is_empty")]
    pub governance: GovernanceMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation_kind: Option<OperationRefusalKind>,
}

impl OperationRefusal {
    pub(crate) fn agent_recoverable(
        reason_code: impl Into<String>,
        governance: GovernanceMetadata,
        recovery_action: impl Into<String>,
    ) -> Self {
        Self {
            reason_code: reason_code.into(),
            recoverability: RefusalRecoverability::AgentRecoverable,
            recovery_action: Some(recovery_action.into()),
            owner_number: None,
            governance,
            escalation_kind: None,
        }
    }

    pub(crate) fn human_required(
        reason_code: impl Into<String>,
        escalation_kind: OperationRefusalKind,
        governance: GovernanceMetadata,
        recovery_action: Option<String>,
    ) -> Self {
        Self {
            reason_code: reason_code.into(),
            recoverability: RefusalRecoverability::HumanRequired,
            recovery_action,
            owner_number: None,
            governance,
            escalation_kind: Some(escalation_kind),
        }
    }

    pub(crate) fn with_owner(mut self, owner_number: Option<u64>) -> Self {
        self.owner_number = owner_number;
        self
    }

    pub(crate) fn escalation_kind(&self) -> Option<OperationRefusalKind> {
        if !self.disposition_is_consistent() {
            return None;
        }
        match self.recoverability {
            RefusalRecoverability::AgentRecoverable => None,
            RefusalRecoverability::HumanRequired => self.escalation_kind,
        }
    }

    fn disposition_is_consistent(&self) -> bool {
        use GovernanceCause::{
            Authority, DomainInvalid, ExternalWait, Integrity, ManagedIdentity, NotReady,
            StructuralGovernance, TransientGovernance,
        };
        if self.reason_code.trim().is_empty()
            || self.governance.effect != Some(GovernanceEffect::Protected)
        {
            return false;
        }
        match self.recoverability {
            RefusalRecoverability::AgentRecoverable => {
                self.escalation_kind.is_none()
                    && self
                        .recovery_action
                        .as_deref()
                        .is_some_and(|action| !action.trim().is_empty())
                    && self.governance.retryable == Some(true)
                    && matches!(
                        self.governance.cause,
                        Some(NotReady | TransientGovernance | ExternalWait)
                    )
            }
            RefusalRecoverability::HumanRequired => {
                self.governance.retryable == Some(false)
                    && matches!(
                        (self.escalation_kind, self.governance.cause),
                        (
                            Some(OperationRefusalKind::Authority),
                            Some(Authority | ManagedIdentity)
                        ) | (Some(OperationRefusalKind::Integrity), Some(Integrity))
                            | (
                                Some(OperationRefusalKind::Immutability),
                                Some(DomainInvalid)
                            )
                            | (
                                Some(OperationRefusalKind::Permission),
                                Some(StructuralGovernance)
                            )
                    )
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct GovernedCommandOutput {
    pub exit_code: i32,
    pub output: String,
    pub refusal: Option<OperationRefusal>,
}

#[derive(Debug)]
pub(crate) struct GovernedCommandFailure {
    pub error: SpecOpsError,
    pub refusal: Option<OperationRefusal>,
}

fn metadata_is_empty(metadata: &GovernanceMetadata) -> bool {
    metadata == &GovernanceMetadata::default()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedOutcome<T> {
    pub outcome: T,
    #[serde(default, skip_serializing_if = "metadata_is_empty")]
    pub governance: GovernanceMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryProbeState {
    Available,
    Satisfied,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryProbe {
    pub operation: String,
    pub state: RecoveryProbeState,
    #[serde(default, skip_serializing_if = "metadata_is_empty")]
    pub governance: GovernanceMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl RecoveryProbe {
    pub(crate) fn available(operation: impl Into<String>, governance: GovernanceMetadata) -> Self {
        Self {
            operation: operation.into(),
            state: RecoveryProbeState::Available,
            governance,
            reason: None,
        }
    }

    pub(crate) fn satisfied(operation: impl Into<String>, governance: GovernanceMetadata) -> Self {
        Self {
            operation: operation.into(),
            state: RecoveryProbeState::Satisfied,
            governance,
            reason: None,
        }
    }

    pub(crate) fn unavailable(
        operation: impl Into<String>,
        governance: GovernanceMetadata,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            operation: operation.into(),
            state: RecoveryProbeState::Unavailable,
            governance,
            reason: Some(reason.into()),
        }
    }

    pub(crate) const fn advertise(&self) -> bool {
        matches!(self.state, RecoveryProbeState::Available)
    }

    pub(crate) const fn executable(&self) -> bool {
        !matches!(self.state, RecoveryProbeState::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum TestOutcome {
        ReboundCurrent,
        SuccessorCreated,
        NotCorrupt,
    }

    #[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
    struct LegacyOutcome {
        outcome: TestOutcome,
    }

    #[test]
    fn governance_metadata_is_additive_and_unknown_field_tolerant() {
        let governed = GovernedOutcome {
            outcome: TestOutcome::SuccessorCreated,
            governance: GovernanceMetadata {
                effect: Some(GovernanceEffect::Protected),
                cause: Some(GovernanceCause::Authority),
                fingerprint: Some("fingerprint-successor".to_string()),
                retryable: Some(false),
                repository_target: Some("repo-identity".to_string()),
                target_state: Some("active".to_string()),
                execution_generation: Some("generation-successor".to_string()),
                audit_id: Some("audit-successor".to_string()),
            },
        };
        let mut value = serde_json::to_value(&governed).expect("serialize governed outcome");
        value
            .as_object_mut()
            .expect("governed object")
            .insert("future_field".to_string(), serde_json::json!({"v": 2}));
        value["governance"]
            .as_object_mut()
            .expect("governance object")
            .insert("future_metadata".to_string(), serde_json::json!(true));

        let decoded: GovernedOutcome<TestOutcome> =
            serde_json::from_value(value.clone()).expect("new reader ignores future fields");
        assert_eq!(decoded, governed);
        let legacy: LegacyOutcome =
            serde_json::from_value(value).expect("legacy reader ignores additive metadata");
        assert_eq!(legacy.outcome, TestOutcome::SuccessorCreated);

        let old = serde_json::json!({"outcome": "rebound_current"});
        let decoded_old: GovernedOutcome<TestOutcome> =
            serde_json::from_value(old).expect("new reader accepts old outcome");
        assert_eq!(decoded_old.outcome, TestOutcome::ReboundCurrent);
        assert_eq!(decoded_old.governance, GovernanceMetadata::default());
    }

    #[test]
    fn governance_metadata_preserves_operation_specific_outcomes() {
        for outcome in [
            TestOutcome::ReboundCurrent,
            TestOutcome::SuccessorCreated,
            TestOutcome::NotCorrupt,
        ] {
            let governed = GovernedOutcome {
                outcome: outcome.clone(),
                governance: GovernanceMetadata::default(),
            };
            let roundtrip: GovernedOutcome<TestOutcome> =
                serde_json::from_value(serde_json::to_value(&governed).expect("serialize outcome"))
                    .expect("deserialize outcome");
            assert_eq!(roundtrip.outcome, outcome);
        }
    }

    #[test]
    fn operation_refusal_roundtrip_keeps_stable_disposition_fields() {
        let refusal = OperationRefusal::agent_recoverable(
            "verification_stale_fingerprint",
            GovernanceMetadata {
                effect: Some(GovernanceEffect::Protected),
                cause: Some(GovernanceCause::NotReady),
                retryable: Some(true),
                ..GovernanceMetadata::default()
            },
            "verify.run",
        );

        let value = serde_json::to_value(&refusal).expect("serialize operation refusal");
        assert_eq!(value["reason_code"], "verification_stale_fingerprint");
        assert_eq!(value["recoverability"], "agent_recoverable");
        assert_eq!(value["recovery_action"], "verify.run");
        assert!(value.get("escalation_kind").is_none());
        assert_eq!(
            serde_json::from_value::<OperationRefusal>(value).expect("roundtrip refusal"),
            refusal
        );
    }

    #[test]
    fn inconsistent_human_disposition_cannot_request_escalation() {
        let refusal = OperationRefusal::human_required(
            "execution_owner_mismatch",
            OperationRefusalKind::Permission,
            GovernanceMetadata {
                effect: Some(GovernanceEffect::Protected),
                cause: Some(GovernanceCause::Authority),
                retryable: Some(false),
                ..GovernanceMetadata::default()
            },
            None,
        );

        assert_eq!(refusal.escalation_kind(), None);
    }
}
