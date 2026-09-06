//! Pure governance value types shared by operation-local evaluators.
//!
//! This module deliberately owns no operation registry, I/O, or durable
//! state. Each operation remains responsible for its own outcome and probe.

use serde::{Deserialize, Serialize};

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
}
